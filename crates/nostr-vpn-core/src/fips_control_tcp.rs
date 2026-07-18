use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config, ConnectionId, MarkerStatus, SendMarker, State};
use fips_tcp_endpoint::FipsTcpEndpoint;
use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::fips_control::{FipsControlFrame, decode_fips_control_frame, encode_fips_control_frame};

/// Reliable nVPN state-control records. Ping/pong remains datagram-based because
/// delayed delivery would make liveness misleading.
pub const FIPS_STATE_CONTROL_SERVICE_PORT: u16 = 7_370;
pub const FIPS_STATE_CONTROL_MAX_RECORD_BYTES: usize = 128 * 1024;

const COMMAND_CAPACITY: usize = 64;
const DELIVERY_CAPACITY: usize = 128;
const MAX_CONNECTIONS: usize = 256;
const MAX_CONNECTIONS_PER_PEER: usize = 8;
const IO_CHUNK_BYTES: usize = 16 * 1024;
const DRIVE_INTERVAL: Duration = Duration::from_millis(20);
const STREAM_TIMEOUT: Duration = Duration::from_secs(15);
static CONTROL_ISN_NONCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct ReceivedFipsControlFrame {
    pub source_peer: PeerIdentity,
    pub frame: FipsControlFrame,
}

struct SendRequest {
    peer: PeerIdentity,
    bytes: Vec<u8>,
    response: Option<oneshot::Sender<std::result::Result<usize, String>>>,
}

struct OutboundRecord {
    bytes: Vec<u8>,
    offset: usize,
    final_marker: Option<SendMarker>,
    started: Instant,
    response: Option<oneshot::Sender<std::result::Result<usize, String>>>,
}

struct InboundRecord {
    peer: PeerIdentity,
    bytes: Vec<u8>,
    started: Instant,
    ready: Option<FipsControlFrame>,
}

/// One state-control record per short FIPS-TCP stream.
///
/// Successful `send` means the authenticated peer's TCP stack acknowledged
/// the complete record. It deliberately does not add an application ACK.
pub struct FipsControlTcpRuntime {
    sender: FipsControlTcpSender,
    received_rx: mpsc::Receiver<ReceivedFipsControlFrame>,
    shutdown: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct FipsControlTcpSender {
    command_tx: mpsc::Sender<SendRequest>,
}

impl FipsControlTcpSender {
    pub async fn send(&self, peer: PeerIdentity, frame: &FipsControlFrame) -> Result<usize> {
        let bytes = encode_stateful_record(frame)?;
        let (response, result) = oneshot::channel();
        self.command_tx
            .send(SendRequest {
                peer,
                bytes,
                response: Some(response),
            })
            .await
            .context("FIPS-TCP state-control runtime stopped before send")?;
        result
            .await
            .context("FIPS-TCP state-control runtime stopped during send")?
            .map_err(anyhow::Error::msg)
    }

    /// Admit a periodic state-control record to the bounded delivery queue
    /// without waiting for the remote TCP acknowledgement. Transactional
    /// callers should keep using [`Self::send`].
    pub fn enqueue(&self, peer: PeerIdentity, frame: &FipsControlFrame) -> Result<usize> {
        let bytes = encode_stateful_record(frame)?;
        let len = bytes.len();
        self.command_tx
            .try_send(SendRequest {
                peer,
                bytes,
                response: None,
            })
            .map_err(|error| anyhow!("FIPS-TCP state-control queue unavailable: {error}"))?;
        Ok(len)
    }
}

fn encode_stateful_record(frame: &FipsControlFrame) -> Result<Vec<u8>> {
    if !is_stateful(frame) {
        return Err(anyhow!("ping/pong must use the FIPS datagram probe path"));
    }
    let bytes = encode_fips_control_frame(frame)?;
    if bytes.len() > FIPS_STATE_CONTROL_MAX_RECORD_BYTES {
        return Err(anyhow!(
            "FIPS state-control record is {} bytes; maximum is {FIPS_STATE_CONTROL_MAX_RECORD_BYTES}",
            bytes.len()
        ));
    }
    Ok(bytes)
}

impl FipsControlTcpRuntime {
    pub async fn start(endpoint: Arc<FipsEndpoint>) -> Result<Self> {
        Self::start_with_delivery_capacity(endpoint, DELIVERY_CAPACITY).await
    }

    async fn start_with_delivery_capacity(
        endpoint: Arc<FipsEndpoint>,
        delivery_capacity: usize,
    ) -> Result<Self> {
        let config = Config {
            receive_buffer: u16::MAX as usize,
            send_buffer: u16::MAX as usize,
            max_connections: MAX_CONNECTIONS,
            max_connections_per_peer: MAX_CONNECTIONS_PER_PEER,
            ..Config::default()
        };
        let tcp = FipsTcpEndpoint::bind(
            Arc::clone(&endpoint),
            FIPS_STATE_CONTROL_SERVICE_PORT,
            config,
            control_isn_seed(endpoint.npub()),
        )
        .await
        .context("failed to bind FIPS-TCP state-control service")?;
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (received_tx, received_rx) = mpsc::channel(delivery_capacity);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run(tcp, command_rx, received_tx, shutdown_rx));
        Ok(Self {
            sender: FipsControlTcpSender { command_tx },
            received_rx,
            shutdown: Some(shutdown_tx),
            task,
        })
    }

    pub async fn send(&self, peer: PeerIdentity, frame: &FipsControlFrame) -> Result<usize> {
        self.sender.send(peer, frame).await
    }

    pub fn sender(&self) -> FipsControlTcpSender {
        self.sender.clone()
    }

    pub async fn recv(&mut self) -> Option<ReceivedFipsControlFrame> {
        self.received_rx.recv().await
    }

    pub fn drain(&mut self) -> Vec<ReceivedFipsControlFrame> {
        let mut received = Vec::new();
        while let Ok(frame) = self.received_rx.try_recv() {
            received.push(frame);
        }
        received
    }

    pub async fn stop(mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        let _ = (&mut self.task).await;
    }
}

impl Drop for FipsControlTcpRuntime {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.abort();
    }
}

async fn run(
    mut tcp: FipsTcpEndpoint,
    mut commands: mpsc::Receiver<SendRequest>,
    received: mpsc::Sender<ReceivedFipsControlFrame>,
    mut shutdown: oneshot::Receiver<()>,
) {
    let mut outbound = HashMap::<ConnectionId, OutboundRecord>::new();
    let mut inbound = HashMap::<ConnectionId, InboundRecord>::new();
    let mut tick = tokio::time::interval(DRIVE_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        let now = now_ms();
        tokio::select! {
            _ = &mut shutdown => break,
            command = commands.recv() => {
                let Some(command) = command else { break; };
                match tcp.connect(command.peer, now).await {
                    Ok(id) => {
                        outbound.insert(id, OutboundRecord {
                            bytes: command.bytes,
                            offset: 0,
                            final_marker: None,
                            started: Instant::now(),
                            response: command.response,
                        });
                    }
                    Err(error) => {
                        if let Some(response) = command.response {
                            let _ = response.send(Err(error.to_string()));
                        }
                    }
                }
            }
            result = tcp.receive_report(now) => {
                if result.is_err() { break; }
            }
            _ = tick.tick() => {
                if tcp.poll(now).await.is_err() { break; }
            }
        }
        drive_ready(&mut tcp, &received, &mut outbound, &mut inbound, now).await;
    }

    for (_, mut record) in outbound {
        if let Some(response) = record.response.take() {
            let _ = response.send(Err("FIPS-TCP state-control runtime stopped".to_string()));
        }
    }
}

async fn drive_ready(
    tcp: &mut FipsTcpEndpoint,
    received: &mpsc::Sender<ReceivedFipsControlFrame>,
    outbound: &mut HashMap<ConnectionId, OutboundRecord>,
    inbound: &mut HashMap<ConnectionId, InboundRecord>,
    now_ms: u64,
) {
    while let Some(id) = tcp.accept() {
        if inbound.len() >= MAX_CONNECTIONS {
            let _ = tcp.abort(id).await;
            continue;
        }
        let Some(peer) = tcp.peer(id) else {
            let _ = tcp.abort(id).await;
            continue;
        };
        inbound.insert(
            id,
            InboundRecord {
                peer,
                bytes: Vec::new(),
                started: Instant::now(),
                ready: None,
            },
        );
    }

    for id in outbound.keys().copied().collect::<Vec<_>>() {
        let Some(mut record) = outbound.remove(&id) else {
            continue;
        };
        let result = drive_outbound(tcp, id, &mut record, now_ms).await;
        match result {
            Ok(Some(sent)) => {
                if let Some(response) = record.response.take() {
                    let _ = response.send(Ok(sent));
                }
            }
            Ok(None) => {
                outbound.insert(id, record);
            }
            Err(error) => {
                let _ = tcp.abort(id).await;
                if let Some(response) = record.response.take() {
                    let _ = response.send(Err(error));
                }
            }
        }
    }

    for id in inbound.keys().copied().collect::<Vec<_>>() {
        let Some(mut record) = inbound.remove(&id) else {
            continue;
        };
        if let Some(frame) = record.ready.take() {
            if !try_deliver(received, &mut record, frame) {
                inbound.insert(id, record);
            }
            continue;
        }
        match drive_inbound(tcp, id, &mut record, now_ms).await {
            Ok(Some(frame)) => {
                if !try_deliver(received, &mut record, frame) {
                    inbound.insert(id, record);
                }
            }
            Ok(None) => {
                inbound.insert(id, record);
            }
            Err(()) => {
                let _ = tcp.abort(id).await;
            }
        }
    }
}

fn try_deliver(
    received: &mpsc::Sender<ReceivedFipsControlFrame>,
    record: &mut InboundRecord,
    frame: FipsControlFrame,
) -> bool {
    match received.try_send(ReceivedFipsControlFrame {
        source_peer: record.peer,
        frame,
    }) {
        Ok(()) | Err(mpsc::error::TrySendError::Closed(_)) => true,
        Err(mpsc::error::TrySendError::Full(message)) => {
            record.ready = Some(message.frame);
            false
        }
    }
}

async fn drive_outbound(
    tcp: &mut FipsTcpEndpoint,
    id: ConnectionId,
    record: &mut OutboundRecord,
    now_ms: u64,
) -> std::result::Result<Option<usize>, String> {
    if record.started.elapsed() >= STREAM_TIMEOUT {
        return Err("FIPS-TCP state-control send timed out".to_string());
    }
    let Some(state) = tcp.state(id) else {
        return Err("FIPS-TCP state-control connection closed before delivery".to_string());
    };
    if matches!(state, State::Established | State::CloseWait) && record.offset < record.bytes.len()
    {
        let end = record
            .offset
            .saturating_add(IO_CHUNK_BYTES)
            .min(record.bytes.len());
        let (accepted, marker) = tcp
            .write_with_marker(id, &record.bytes[record.offset..end], now_ms)
            .await
            .map_err(|error| error.to_string())?;
        record.offset = record.offset.saturating_add(accepted);
        if accepted > 0 && record.offset == record.bytes.len() {
            record.final_marker = Some(marker);
        }
    }
    let Some(marker) = record.final_marker.as_ref() else {
        return Ok(None);
    };
    match tcp.marker_status(marker) {
        MarkerStatus::Pending => Ok(None),
        MarkerStatus::ConnectionGone => {
            Err("FIPS-TCP state-control connection closed before acknowledgment".to_string())
        }
        MarkerStatus::Acked => {
            tcp.close(id, now_ms)
                .await
                .map_err(|error| error.to_string())?;
            Ok(Some(record.bytes.len()))
        }
    }
}

async fn drive_inbound(
    tcp: &mut FipsTcpEndpoint,
    id: ConnectionId,
    record: &mut InboundRecord,
    now_ms: u64,
) -> std::result::Result<Option<FipsControlFrame>, ()> {
    if record.started.elapsed() >= STREAM_TIMEOUT || tcp.state(id).is_none() {
        return Err(());
    }
    if matches!(tcp.state(id), Some(State::Established | State::CloseWait)) {
        let remaining = FIPS_STATE_CONTROL_MAX_RECORD_BYTES.saturating_sub(record.bytes.len());
        if remaining == 0 {
            return Err(());
        }
        let bytes = tcp
            .read(id, remaining.min(IO_CHUNK_BYTES), now_ms)
            .await
            .map_err(|_| ())?;
        record.bytes.extend(bytes);
        if let Some(frame) = decode_complete_stateful_record(&record.bytes) {
            tcp.close(id, now_ms).await.map_err(|_| ())?;
            return Ok(Some(frame));
        }
    }
    if !tcp.is_read_closed(id) {
        return Ok(None);
    }
    tcp.close(id, now_ms).await.map_err(|_| ())?;
    let frame = decode_complete_stateful_record(&record.bytes).ok_or(())?;
    Ok(Some(frame))
}

fn decode_complete_stateful_record(bytes: &[u8]) -> Option<FipsControlFrame> {
    decode_fips_control_frame(bytes)
        .ok()
        .flatten()
        .filter(is_stateful)
}

fn is_stateful(frame: &FipsControlFrame) -> bool {
    !matches!(
        frame,
        FipsControlFrame::Ping { .. } | FipsControlFrame::Pong { .. }
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn control_isn_seed(npub: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(npub.as_bytes());
    hasher.update(now_ms().to_be_bytes());
    hasher.update(
        CONTROL_ISN_NONCE
            .fetch_add(1, Ordering::Relaxed)
            .to_be_bytes(),
    );
    let digest = hasher.finalize();
    u64::from_be_bytes(
        digest[..8]
            .try_into()
            .expect("SHA-256 prefix is eight bytes"),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use fips_core::{FipsEndpoint, PeerIdentity};

    use super::*;

    #[tokio::test]
    async fn carries_one_stateful_record_over_real_fips_tcp() {
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind embedded FIPS endpoint"),
        );
        let local = PeerIdentity::from_npub(endpoint.npub()).expect("local peer identity");
        let mut control = FipsControlTcpRuntime::start(Arc::clone(&endpoint))
            .await
            .expect("start state-control runtime");
        let frame = FipsControlFrame::Capabilities {
            network_id: "network".to_string(),
            capabilities: Default::default(),
        };

        let sent = tokio::time::timeout(Duration::from_secs(3), control.send(local, &frame))
            .await
            .expect("state-control send timed out")
            .expect("send state-control frame");
        assert_eq!(
            sent,
            encode_fips_control_frame(&frame).expect("encode").len()
        );
        let received = tokio::time::timeout(Duration::from_secs(3), control.recv())
            .await
            .expect("state-control receive timed out")
            .expect("receive state-control frame");
        assert_eq!(received.source_peer, local);
        assert_eq!(received.frame, frame);

        control.stop().await;
        endpoint.shutdown().await.expect("shutdown endpoint");
    }

    #[tokio::test]
    async fn enqueue_returns_before_stateful_delivery_completes() {
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind embedded FIPS endpoint"),
        );
        let local = PeerIdentity::from_npub(endpoint.npub()).expect("local peer identity");
        let mut control = FipsControlTcpRuntime::start(Arc::clone(&endpoint))
            .await
            .expect("start state-control runtime");
        let frame = FipsControlFrame::Capabilities {
            network_id: "network".to_string(),
            capabilities: Default::default(),
        };

        let started = Instant::now();
        let queued = control
            .sender()
            .enqueue(local, &frame)
            .expect("queue frame");
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "queue admission must not wait for FIPS-TCP acknowledgement"
        );
        assert_eq!(
            queued,
            encode_fips_control_frame(&frame).expect("encode").len()
        );
        let received = tokio::time::timeout(Duration::from_secs(3), control.recv())
            .await
            .expect("state-control receive timed out")
            .expect("receive queued state-control frame");
        assert_eq!(received.frame, frame);

        control.stop().await;
        endpoint.shutdown().await.expect("shutdown endpoint");
    }

    #[tokio::test]
    async fn rejects_ping_on_the_stateful_stream() {
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind embedded FIPS endpoint"),
        );
        let local = PeerIdentity::from_npub(endpoint.npub()).expect("local peer identity");
        let control = FipsControlTcpRuntime::start(Arc::clone(&endpoint))
            .await
            .expect("start state-control runtime");
        let error = control
            .send(
                local,
                &FipsControlFrame::Ping {
                    network_id: "network".to_string(),
                    sent_at: 1,
                },
            )
            .await
            .expect_err("ping must stay on datagram path");
        assert!(error.to_string().contains("datagram probe"));
        control.stop().await;
        endpoint.shutdown().await.expect("shutdown endpoint");
    }

    #[test]
    fn complete_record_is_ready_without_waiting_for_stream_close() {
        let frame = FipsControlFrame::Capabilities {
            network_id: "network".to_string(),
            capabilities: Default::default(),
        };
        let bytes = encode_fips_control_frame(&frame).expect("encode state-control frame");
        assert_eq!(decode_complete_stateful_record(&bytes), Some(frame));
        assert!(decode_complete_stateful_record(&bytes[..bytes.len() - 1]).is_none());
    }

    #[tokio::test]
    async fn retains_a_record_while_the_local_delivery_queue_is_full() {
        let endpoint = Arc::new(
            FipsEndpoint::builder()
                .without_system_tun()
                .bind()
                .await
                .expect("bind embedded FIPS endpoint"),
        );
        let local = PeerIdentity::from_npub(endpoint.npub()).expect("local peer identity");
        let mut control =
            FipsControlTcpRuntime::start_with_delivery_capacity(Arc::clone(&endpoint), 1)
                .await
                .expect("start state-control runtime");

        for index in 0..=1 {
            let frame = FipsControlFrame::Capabilities {
                network_id: index.to_string(),
                capabilities: Default::default(),
            };
            tokio::time::timeout(Duration::from_secs(3), control.send(local, &frame))
                .await
                .expect("state-control send timed out")
                .expect("send state-control frame");
        }
        for index in 0..=1 {
            let received = tokio::time::timeout(Duration::from_secs(3), control.recv())
                .await
                .expect("state-control receive timed out")
                .expect("receive state-control frame");
            let FipsControlFrame::Capabilities { network_id, .. } = received.frame else {
                panic!("unexpected state-control frame");
            };
            assert_eq!(network_id, index.to_string());
        }

        control.stop().await;
        endpoint.shutdown().await.expect("shutdown endpoint");
    }

    #[test]
    fn restart_uses_a_fresh_tcp_sequence_seed() {
        let first = control_isn_seed("npub-test");
        let second = control_isn_seed("npub-test");
        assert_ne!(first, second);
    }
}
