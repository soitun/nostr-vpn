use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fips_core::{FipsEndpointServiceDatagram, FipsEndpointServiceReceiver};
use fips_endpoint::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config, ConnectionId, Stack, State};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

const COMMAND_CAPACITY: usize = 64;
const DELIVERY_CAPACITY: usize = 256;
const MAX_PENDING_RECORDS: usize = 1_024;
const MAX_PENDING_RECORDS_PER_PEER: usize = 64;
const MAX_PENDING_DELIVERIES: usize = 1_024;
const DRIVER_TICK: Duration = Duration::from_millis(25);
const RECEIVE_BATCH: usize = 64;

#[derive(Debug)]
pub(crate) enum FipsTcpRecordEvent {
    Connected {
        peer: String,
    },
    Record {
        source_peer: String,
        payload: Vec<u8>,
    },
}

struct SendRecord {
    peer: String,
    payload: Vec<u8>,
    response: oneshot::Sender<Result<()>>,
}

pub(crate) struct FipsTcpRecordTransport {
    commands: mpsc::Sender<SendRecord>,
    received: mpsc::Receiver<FipsTcpRecordEvent>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub(crate) struct FipsTcpRecordSender {
    commands: mpsc::Sender<SendRecord>,
}

impl FipsTcpRecordTransport {
    pub(crate) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        service_port: u16,
        max_record_bytes: usize,
    ) -> Result<Self> {
        if max_record_bytes == 0 || max_record_bytes.saturating_add(4) > u16::MAX as usize {
            return Err(anyhow!("invalid TCP/FIPS record limit {max_record_bytes}"));
        }
        let receiver = endpoint
            .register_service_receiver(service_port)
            .await
            .context("failed to register TCP/FIPS record service")?;
        let mut stack = Stack::new(Config::default(), isn_seed(endpoint.npub(), service_port));
        stack.listen(service_port)?;
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (received_tx, received) = mpsc::channel(DELIVERY_CAPACITY);
        let task = tokio::spawn(run_driver(
            Driver::new(endpoint, stack, service_port, max_record_bytes, received_tx),
            receiver,
            command_rx,
        ));
        Ok(Self {
            commands: command_tx,
            received,
            task,
        })
    }

    pub(crate) fn sender(&self) -> FipsTcpRecordSender {
        FipsTcpRecordSender {
            commands: self.commands.clone(),
        }
    }

    pub(crate) async fn recv(&mut self) -> Option<FipsTcpRecordEvent> {
        self.received.recv().await
    }

    pub(crate) async fn stop(mut self) {
        self.task.abort();
        let _ = (&mut self.task).await;
    }
}

impl FipsTcpRecordSender {
    pub(crate) async fn send(&self, peer: &str, payload: Vec<u8>) -> Result<()> {
        let (response, result) = oneshot::channel();
        self.commands
            .send(SendRecord {
                peer: peer.to_string(),
                payload,
                response,
            })
            .await
            .context("TCP/FIPS record transport stopped before send")?;
        result
            .await
            .context("TCP/FIPS record transport stopped during send")?
    }
}

impl Drop for FipsTcpRecordTransport {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ActiveWrite {
    peer: String,
    frame: Vec<u8>,
    accepted: usize,
}

impl ActiveWrite {
    fn remaining(&self) -> &[u8] {
        &self.frame[self.accepted..]
    }
}

#[derive(Default)]
struct RecordQueues {
    pending: HashMap<String, VecDeque<Vec<u8>>>,
    active: HashMap<ConnectionId, ActiveWrite>,
    count: usize,
}

impl RecordQueues {
    fn enqueue(&mut self, peer: String, frame: Vec<u8>) -> Result<()> {
        let peer_count = self.pending.get(&peer).map_or(0, VecDeque::len)
            + self
                .active
                .values()
                .filter(|write| write.peer == peer)
                .count();
        if self.count >= MAX_PENDING_RECORDS || peer_count >= MAX_PENDING_RECORDS_PER_PEER {
            return Err(anyhow!("TCP/FIPS record queue is full"));
        }
        self.pending.entry(peer).or_default().push_back(frame);
        self.count += 1;
        Ok(())
    }

    fn pending_peers(&self) -> Vec<String> {
        self.pending.keys().cloned().collect()
    }

    fn stage_next(&mut self, peer: &str, id: ConnectionId) {
        if self.active.contains_key(&id) {
            return;
        }
        let Some(frame) = self.pending.get_mut(peer).and_then(VecDeque::pop_front) else {
            return;
        };
        self.pending.retain(|_, records| !records.is_empty());
        self.active.insert(
            id,
            ActiveWrite {
                peer: peer.to_string(),
                frame,
                accepted: 0,
            },
        );
    }

    fn note_accepted(&mut self, id: ConnectionId, accepted: usize) {
        let Some(write) = self.active.get_mut(&id) else {
            return;
        };
        write.accepted = write
            .accepted
            .saturating_add(accepted)
            .min(write.frame.len());
        if write.accepted == write.frame.len() {
            self.active.remove(&id);
            self.count = self.count.saturating_sub(1);
        }
    }

    fn retain_connections(&mut self, mut retained: impl FnMut(ConnectionId) -> bool) {
        let closed = self
            .active
            .keys()
            .copied()
            .filter(|id| !retained(*id))
            .collect::<Vec<_>>();
        for id in closed {
            if let Some(write) = self.active.remove(&id) {
                self.pending
                    .entry(write.peer)
                    .or_default()
                    .push_front(write.frame);
            }
        }
    }
}

struct Driver {
    endpoint: Arc<FipsEndpoint>,
    stack: Stack<String>,
    service_port: u16,
    max_record_bytes: usize,
    connections: HashSet<ConnectionId>,
    peers: PeerConnections,
    records: RecordQueues,
    reads: HashMap<ConnectionId, Vec<u8>>,
    deliveries: VecDeque<FipsTcpRecordEvent>,
    delivered: mpsc::Sender<FipsTcpRecordEvent>,
}

impl Driver {
    fn new(
        endpoint: Arc<FipsEndpoint>,
        stack: Stack<String>,
        service_port: u16,
        max_record_bytes: usize,
        delivered: mpsc::Sender<FipsTcpRecordEvent>,
    ) -> Self {
        Self {
            endpoint,
            stack,
            service_port,
            max_record_bytes,
            connections: HashSet::new(),
            peers: PeerConnections::default(),
            records: RecordQueues::default(),
            reads: HashMap::new(),
            deliveries: VecDeque::new(),
            delivered,
        }
    }

    fn queue_record(&mut self, peer: String, payload: Vec<u8>, now_ms: u64) -> Result<()> {
        PeerIdentity::from_npub(&peer).context("invalid TCP/FIPS record peer")?;
        if payload.len() > self.max_record_bytes {
            return Err(anyhow!(
                "TCP/FIPS record is {} bytes; limit is {}",
                payload.len(),
                self.max_record_bytes
            ));
        }
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(&payload);
        self.records.enqueue(peer.clone(), frame)?;
        self.peers.desired.insert(peer.clone());
        self.ensure_connection(&peer, now_ms)
    }

    fn ensure_connection(&mut self, peer: &str, now_ms: u64) -> Result<()> {
        if self.peers.active.get(peer).is_some_and(|id| {
            matches!(
                self.stack.state(*id),
                Some(State::Established | State::CloseWait | State::SynSent)
            )
        }) {
            return Ok(());
        }
        if let Some(id) = self.connections.iter().copied().find(|id| {
            self.stack
                .peer(*id)
                .is_some_and(|candidate| candidate == peer)
                && matches!(
                    self.stack.state(*id),
                    Some(State::Established | State::CloseWait | State::SynSent)
                )
        }) {
            self.peers.active.insert(peer.to_string(), id);
            return Ok(());
        }
        let id = self
            .stack
            .connect(peer.to_string(), self.service_port, now_ms)?;
        self.connections.insert(id);
        self.peers.active.insert(peer.to_string(), id);
        Ok(())
    }

    fn input(&mut self, datagrams: &[FipsEndpointServiceDatagram], now_ms: u64) {
        for datagram in datagrams {
            let peer = datagram.source_peer.npub();
            if let Err(error) = self
                .stack
                .input(peer.clone(), datagram.data.as_ref(), now_ms)
            {
                tracing::debug!(%error, %peer, "ignored invalid TCP/FIPS segment");
            }
        }
    }

    async fn service(&mut self, now_ms: u64) {
        self.accept_connections();
        self.remove_closed_connections(now_ms);
        self.select_established_connections();
        self.flush_pending_writes(now_ms);
        self.collect_records(now_ms);
        self.flush_outbound().await;
        self.flush_deliveries();
    }

    fn accept_connections(&mut self) {
        while let Some(id) = self.stack.accept(self.service_port) {
            self.connections.insert(id);
        }
    }

    fn remove_closed_connections(&mut self, now_ms: u64) {
        self.connections
            .retain(|id| self.stack.state(*id).is_some());
        self.peers
            .announced
            .retain(|id| self.connections.contains(id));
        self.peers
            .active
            .retain(|_, id| self.connections.contains(id) && self.stack.state(*id).is_some());
        self.records
            .retain_connections(|id| self.connections.contains(&id));
        self.reads.retain(|id, _| self.connections.contains(id));
        let peers = self.peers.desired.iter().cloned().collect::<Vec<_>>();
        for peer in peers {
            if let Err(error) = self.ensure_connection(&peer, now_ms) {
                tracing::debug!(%error, %peer, "TCP/FIPS peer reconnect deferred");
            }
        }
    }

    fn select_established_connections(&mut self) {
        self.peers
            .select_established(&self.stack, &self.connections, &mut self.deliveries);
    }

    fn flush_pending_writes(&mut self, now_ms: u64) {
        let peers = self.records.pending_peers();
        for peer in peers {
            let Some(id) = self.peers.active.get(&peer).copied() else {
                continue;
            };
            if !matches!(
                self.stack.state(id),
                Some(State::Established | State::CloseWait)
            ) {
                continue;
            }
            self.records.stage_next(&peer, id);
            let Some(write) = self.records.active.get(&id) else {
                continue;
            };
            match self.stack.write(id, write.remaining(), now_ms) {
                Ok(accepted) => {
                    self.records.note_accepted(id, accepted);
                }
                Err(error) => {
                    tracing::debug!(%error, %peer, "TCP/FIPS stream write deferred");
                }
            }
        }
    }

    fn collect_records(&mut self, now_ms: u64) {
        if self.deliveries.len() >= MAX_PENDING_DELIVERIES {
            return;
        }
        let ids = self.connections.iter().copied().collect::<Vec<_>>();
        for id in ids {
            if self.deliveries.len() >= MAX_PENDING_DELIVERIES
                || !matches!(
                    self.stack.state(id),
                    Some(State::Established | State::CloseWait)
                )
            {
                continue;
            }
            let Some(peer) = self.stack.peer(id).cloned() else {
                continue;
            };
            let bytes = match self.stack.read(id, u16::MAX as usize, now_ms) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::debug!(%error, %peer, "TCP/FIPS stream read failed");
                    continue;
                }
            };
            if bytes.is_empty() {
                continue;
            }
            let buffer = self.reads.entry(id).or_default();
            buffer.extend_from_slice(&bytes);
            while let Some(length_bytes) = buffer.get(..4) {
                let length =
                    u32::from_be_bytes(length_bytes.try_into().expect("four bytes")) as usize;
                if length > self.max_record_bytes {
                    tracing::debug!(%peer, length, "closed oversized TCP/FIPS record stream");
                    buffer.clear();
                    let _ = self.stack.close(id, now_ms);
                    break;
                }
                let frame_len = length + 4;
                if buffer.len() < frame_len {
                    break;
                }
                let payload = buffer[4..frame_len].to_vec();
                buffer.drain(..frame_len);
                self.deliveries.push_back(FipsTcpRecordEvent::Record {
                    source_peer: peer.clone(),
                    payload,
                });
                if self.deliveries.len() >= MAX_PENDING_DELIVERIES {
                    break;
                }
            }
        }
    }

    async fn flush_outbound(&mut self) {
        for outbound in self.stack.drain_outbound() {
            let peer = match PeerIdentity::from_npub(&outbound.peer) {
                Ok(peer) => peer,
                Err(error) => {
                    tracing::debug!(%error, peer = %outbound.peer, "ignored invalid TCP/FIPS peer");
                    continue;
                }
            };
            if let Err(error) = self
                .endpoint
                .send_datagram(peer, self.service_port, self.service_port, outbound.bytes)
                .await
            {
                tracing::debug!(%error, peer = %outbound.peer, "TCP/FIPS segment send failed; retransmission remains armed");
            }
        }
    }

    fn flush_deliveries(&mut self) {
        while let Some(record) = self.deliveries.pop_front() {
            match self.delivered.try_send(record) {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(record)) => {
                    self.deliveries.push_front(record);
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => break,
            }
        }
    }
}

#[derive(Default)]
struct PeerConnections {
    announced: HashSet<ConnectionId>,
    desired: HashSet<String>,
    active: HashMap<String, ConnectionId>,
}

impl PeerConnections {
    fn select_established(
        &mut self,
        stack: &Stack<String>,
        connections: &HashSet<ConnectionId>,
        deliveries: &mut VecDeque<FipsTcpRecordEvent>,
    ) {
        for id in connections.iter().copied() {
            if !matches!(stack.state(id), Some(State::Established | State::CloseWait)) {
                continue;
            }
            let Some(peer) = stack.peer(id).cloned() else {
                continue;
            };
            let replace = self.active.get(&peer).is_none_or(|active| {
                !matches!(
                    stack.state(*active),
                    Some(State::Established | State::CloseWait)
                )
            });
            if replace {
                self.active.insert(peer.clone(), id);
            }
            if self.announced.insert(id) {
                deliveries.push_back(FipsTcpRecordEvent::Connected { peer });
            }
        }
    }
}

async fn run_driver(
    mut driver: Driver,
    receiver: FipsEndpointServiceReceiver,
    mut commands: mpsc::Receiver<SendRecord>,
) {
    let mut datagrams = Vec::with_capacity(RECEIVE_BATCH);
    let mut tick = tokio::time::interval(DRIVER_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break; };
                let result = driver.queue_record(command.peer, command.payload, now_ms());
                let _ = command.response.send(result);
            }
            count = receiver.recv_batch_into(&mut datagrams, RECEIVE_BATCH) => {
                let Some(count) = count else { break; };
                driver.input(&datagrams[..count], now_ms());
                datagrams.clear();
            }
            _ = tick.tick() => {
                driver.stack.poll(now_ms());
            }
        }
        driver.service(now_ms()).await;
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn isn_seed(npub: &str, service_port: u16) -> u64 {
    let mut hasher = DefaultHasher::new();
    npub.hash(&mut hasher);
    service_port.hash(&mut hasher);
    now_ms().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(client: &mut Stack<String>, server: &mut Stack<String>, now_ms: u64) {
        for _ in 0..32 {
            let client_segments = client.drain_outbound();
            let server_segments = server.drain_outbound();
            if client_segments.is_empty() && server_segments.is_empty() {
                return;
            }
            for segment in client_segments {
                server
                    .input("client".to_string(), &segment.bytes, now_ms)
                    .expect("server accepts client segment");
            }
            for segment in server_segments {
                client
                    .input("server".to_string(), &segment.bytes, now_ms)
                    .expect("client accepts server segment");
            }
        }
        panic!("TCP/FIPS handshake did not settle");
    }

    #[test]
    fn inbound_connection_is_announced_without_becoming_reconnect_intent() {
        let mut client = Stack::new(Config::default(), 1);
        let mut server = Stack::new(Config::default(), 2);
        server.listen(7369).expect("listen");
        client
            .connect("server".to_string(), 7369, 0)
            .expect("connect");
        settle(&mut client, &mut server, 0);
        let inbound = server.accept(7369).expect("accept inbound connection");

        let connections = HashSet::from([inbound]);
        let mut peers = PeerConnections::default();
        let mut deliveries = VecDeque::new();
        peers.select_established(&server, &connections, &mut deliveries);

        assert_eq!(peers.active.get("client"), Some(&inbound));
        assert!(peers.announced.contains(&inbound));
        assert!(matches!(
            deliveries.pop_front(),
            Some(FipsTcpRecordEvent::Connected { peer }) if peer == "client"
        ));
        assert!(
            peers.desired.is_empty(),
            "inbound peers are not reconnect intent"
        );
    }

    #[test]
    fn active_partial_write_stays_charged_and_bounded() {
        let peer = "peer".to_string();
        let mut queues = RecordQueues::default();
        for marker in 0..MAX_PENDING_RECORDS_PER_PEER {
            queues
                .enqueue(peer.clone(), vec![marker as u8; 32])
                .expect("queue bounded record");
        }

        let id = ConnectionId::from_raw(1);
        queues.stage_next(&peer, id);
        assert_eq!(queues.count, MAX_PENDING_RECORDS_PER_PEER);
        assert_eq!(queues.active.len(), 1);
        assert_eq!(
            queues.pending[&peer].len(),
            MAX_PENDING_RECORDS_PER_PEER - 1
        );
        assert!(queues.enqueue(peer.clone(), vec![0; 32]).is_err());

        queues.note_accepted(id, 16);
        assert_eq!(queues.count, MAX_PENDING_RECORDS_PER_PEER);
        assert_eq!(queues.active[&id].remaining().len(), 16);
        assert!(queues.enqueue(peer.clone(), vec![0; 32]).is_err());

        queues.note_accepted(id, 16);
        assert_eq!(queues.count, MAX_PENDING_RECORDS_PER_PEER - 1);
        assert!(!queues.active.contains_key(&id));
        queues
            .enqueue(peer, vec![0; 32])
            .expect("completed record releases one bounded slot");
    }
}
