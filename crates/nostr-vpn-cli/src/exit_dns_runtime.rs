use std::collections::{HashMap, HashSet};
#[cfg(any(target_os = "linux", test))]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
#[cfg(not(any(target_os = "linux", test)))]
use std::time::Duration;
#[cfg(any(target_os = "linux", test))]
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(any(target_os = "linux", test))]
use anyhow::anyhow;
use anyhow::{Context, Result};
use fips_core::{FipsEndpoint, FipsEndpointServiceReceiver, PeerIdentity};
use nostr_vpn_core::exit_dns::{
    EXIT_DNS_FIPS_SERVICE_PORT, ExitDnsMessage, ExitDnsRequest, ExitDnsResponse,
    build_exit_dns_servfail_response, decode_exit_dns_message, encode_exit_dns_message,
};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};
use tokio::task::{JoinHandle, JoinSet};

use crate::exit_dns_resolver::HostDnsResolver;

const EXIT_DNS_RECEIVE_BATCH: usize = 16;
const EXIT_DNS_MAX_IN_FLIGHT: usize = 16;
#[cfg(any(target_os = "linux", test))]
const EXIT_DNS_MAX_PENDING: usize = 64;
const EXIT_DNS_CLIENT_TIMEOUT: Duration = Duration::from_secs(3);

type PendingKey = (String, u64, u16);
type PendingRequests = Arc<Mutex<HashMap<PendingKey, oneshot::Sender<Vec<u8>>>>>;

#[derive(Clone)]
pub(crate) struct ExitDnsFipsClient {
    #[cfg(any(target_os = "linux", test))]
    endpoint: Arc<FipsEndpoint>,
    pending: PendingRequests,
    #[cfg(any(target_os = "linux", test))]
    next_nonce: Arc<AtomicU64>,
    #[cfg(any(target_os = "linux", test))]
    timeout: Duration,
}

impl ExitDnsFipsClient {
    #[cfg(any(target_os = "linux", test))]
    pub(crate) async fn resolve(&self, exit_peer: PeerIdentity, query: &[u8]) -> Result<Vec<u8>> {
        let nonce = self.next_nonce.fetch_add(1, Ordering::Relaxed);
        let request = ExitDnsRequest::new(nonce, query.to_vec())
            .context("invalid DNS query for exit service")?;
        let key = (exit_peer.npub(), request.nonce, request.transaction_id);
        let payload = encode_exit_dns_message(&ExitDnsMessage::Request(request))
            .context("failed to encode exit DNS request")?;
        let (response_tx, response_rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            if pending.len() >= EXIT_DNS_MAX_PENDING {
                return Err(anyhow!("too many pending exit DNS requests"));
            }
            pending.insert(key.clone(), response_tx);
        }

        if let Err(error) = self
            .endpoint
            .send_datagram(
                exit_peer,
                EXIT_DNS_FIPS_SERVICE_PORT,
                EXIT_DNS_FIPS_SERVICE_PORT,
                payload,
            )
            .await
        {
            self.pending.lock().await.remove(&key);
            return Err(error).context("failed to send exit DNS request over FIPS");
        }

        match tokio::time::timeout(self.timeout, response_rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(anyhow!("exit DNS response channel closed")),
            Err(_) => {
                self.pending.lock().await.remove(&key);
                Err(anyhow!("exit DNS request timed out"))
            }
        }
    }

    async fn deliver(&self, source: PeerIdentity, response: ExitDnsResponse) {
        let key = (source.npub(), response.nonce, response.transaction_id);
        if let Some(sender) = self.pending.lock().await.remove(&key) {
            let _ = sender.send(response.response);
        }
    }
}

#[derive(Debug, Default)]
struct ExitDnsServiceState {
    enabled: bool,
    authorized_peers: HashSet<String>,
}

#[derive(Clone, Default)]
struct ExitDnsServicePolicy {
    state: Arc<RwLock<ExitDnsServiceState>>,
}

impl ExitDnsServicePolicy {
    fn reconfigure(&self, enabled: bool, authorized_peers: &[String]) {
        let mut state = self
            .state
            .write()
            .unwrap_or_else(|error| error.into_inner());
        state.enabled = enabled;
        state.authorized_peers.clear();
        state
            .authorized_peers
            .extend(authorized_peers.iter().cloned());
    }

    fn permits(&self, source_npub: &str) -> bool {
        let state = self.state.read().unwrap_or_else(|error| error.into_inner());
        state.enabled && state.authorized_peers.contains(source_npub)
    }
}

pub(crate) struct ExitDnsFipsRuntime {
    #[cfg(any(target_os = "linux", test))]
    client: ExitDnsFipsClient,
    policy: ExitDnsServicePolicy,
    command_tx: mpsc::Sender<()>,
    task: Option<JoinHandle<()>>,
}

impl ExitDnsFipsRuntime {
    #[cfg(target_os = "linux")]
    pub(crate) async fn start_client(endpoint: Arc<FipsEndpoint>) -> Result<Self> {
        Self::start(endpoint, None, false, &[], EXIT_DNS_CLIENT_TIMEOUT).await
    }

    pub(crate) async fn start_tunnel(
        endpoint: Arc<FipsEndpoint>,
        enabled: bool,
        authorized_peers: &[String],
    ) -> Result<Self> {
        Self::start(
            endpoint,
            Some(HostDnsResolver::system()),
            enabled,
            authorized_peers,
            EXIT_DNS_CLIENT_TIMEOUT,
        )
        .await
    }

    async fn start(
        endpoint: Arc<FipsEndpoint>,
        resolver: Option<HostDnsResolver>,
        enabled: bool,
        authorized_peers: &[String],
        client_timeout: Duration,
    ) -> Result<Self> {
        let receiver = endpoint
            .register_service_receiver(EXIT_DNS_FIPS_SERVICE_PORT)
            .await
            .context("failed to register the FIPS exit DNS service")?;
        #[cfg(not(any(target_os = "linux", test)))]
        let _ = client_timeout;
        let client = ExitDnsFipsClient {
            #[cfg(any(target_os = "linux", test))]
            endpoint: Arc::clone(&endpoint),
            pending: Arc::new(Mutex::new(HashMap::new())),
            #[cfg(any(target_os = "linux", test))]
            next_nonce: Arc::new(AtomicU64::new(initial_nonce())),
            #[cfg(any(target_os = "linux", test))]
            timeout: client_timeout,
        };
        let policy = ExitDnsServicePolicy::default();
        policy.reconfigure(enabled, authorized_peers);
        let (command_tx, command_rx) = mpsc::channel(1);
        let task_client = client.clone();
        let task_policy = policy.clone();
        let task = tokio::spawn(async move {
            run_exit_dns_service(
                endpoint,
                receiver,
                resolver,
                task_policy,
                task_client,
                command_rx,
            )
            .await;
        });
        Ok(Self {
            #[cfg(any(target_os = "linux", test))]
            client,
            policy,
            command_tx,
            task: Some(task),
        })
    }

    #[cfg(test)]
    async fn start_for_test(
        endpoint: Arc<FipsEndpoint>,
        resolver: Option<HostDnsResolver>,
        enabled: bool,
        authorized_peers: &[String],
        client_timeout: Duration,
    ) -> Result<Self> {
        Self::start(
            endpoint,
            resolver,
            enabled,
            authorized_peers,
            client_timeout,
        )
        .await
    }

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn client(&self) -> ExitDnsFipsClient {
        self.client.clone()
    }

    pub(crate) fn reconfigure(&self, enabled: bool, authorized_peers: &[String]) {
        self.policy.reconfigure(enabled, authorized_peers);
    }

    pub(crate) async fn stop(mut self) {
        let _ = self.command_tx.send(()).await;
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}

impl Drop for ExitDnsFipsRuntime {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

async fn run_exit_dns_service(
    endpoint: Arc<FipsEndpoint>,
    receiver: FipsEndpointServiceReceiver,
    resolver: Option<HostDnsResolver>,
    policy: ExitDnsServicePolicy,
    client: ExitDnsFipsClient,
    mut command_rx: mpsc::Receiver<()>,
) {
    let mut datagrams = Vec::with_capacity(EXIT_DNS_RECEIVE_BATCH);
    let permits = Arc::new(Semaphore::new(EXIT_DNS_MAX_IN_FLIGHT));
    let mut requests = JoinSet::new();
    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let _ = command;
                break;
            }
            completed = requests.join_next(), if !requests.is_empty() => {
                if let Some(Err(error)) = completed {
                    tracing::debug!(%error, "exit DNS request task failed");
                }
            }
            count = receiver.recv_batch_into(&mut datagrams, EXIT_DNS_RECEIVE_BATCH) => {
                let Some(count) = count else { break; };
                for datagram in datagrams.iter().take(count) {
                    if datagram.source_port != EXIT_DNS_FIPS_SERVICE_PORT
                        || datagram.destination_port != EXIT_DNS_FIPS_SERVICE_PORT
                    {
                        continue;
                    }
                    let message = match decode_exit_dns_message(datagram.data.as_ref()) {
                        Ok(message) => message,
                        Err(error) => {
                            tracing::debug!(%error, source = %datagram.source_peer, "ignored invalid exit DNS datagram");
                            continue;
                        }
                    };
                    match message {
                        ExitDnsMessage::Request(request) => {
                            let source_npub = datagram.source_peer.npub();
                            if !policy.permits(&source_npub) {
                                send_exit_dns_response(
                                    &endpoint,
                                    datagram.source_peer,
                                    servfail_exit_dns_response(&request),
                                )
                                .await;
                                continue;
                            }
                            let Ok(permit) = Arc::clone(&permits).try_acquire_owned() else {
                                send_exit_dns_response(
                                    &endpoint,
                                    datagram.source_peer,
                                    servfail_exit_dns_response(&request),
                                )
                                .await;
                                continue;
                            };
                            let request_endpoint = Arc::clone(&endpoint);
                            let request_policy = policy.clone();
                            let request_resolver = resolver.clone();
                            let source_peer = datagram.source_peer;
                            requests.spawn(async move {
                                let _permit = permit;
                                let response = answer_exit_dns_request(
                                    &source_npub,
                                    request,
                                    &request_policy,
                                    request_resolver.as_ref(),
                                )
                                .await;
                                send_exit_dns_response(&request_endpoint, source_peer, response).await;
                            });
                        }
                        ExitDnsMessage::Response(response) => {
                            client.deliver(datagram.source_peer, response).await;
                        }
                    }
                }
                datagrams.clear();
            }
        }
    }
    requests.abort_all();
    while requests.join_next().await.is_some() {}
}

async fn send_exit_dns_response(
    endpoint: &FipsEndpoint,
    destination: PeerIdentity,
    response: ExitDnsResponse,
) {
    let payload = match encode_exit_dns_message(&ExitDnsMessage::Response(response)) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(%error, "failed to encode exit DNS response");
            return;
        }
    };
    if let Err(error) = endpoint
        .send_datagram(
            destination,
            EXIT_DNS_FIPS_SERVICE_PORT,
            EXIT_DNS_FIPS_SERVICE_PORT,
            payload,
        )
        .await
    {
        tracing::debug!(%error, source = %destination, "failed to return exit DNS response");
    }
}

async fn answer_exit_dns_request(
    source_npub: &str,
    request: ExitDnsRequest,
    policy: &ExitDnsServicePolicy,
    resolver: Option<&HostDnsResolver>,
) -> ExitDnsResponse {
    let response = if policy.permits(source_npub) {
        match resolver {
            Some(resolver) => {
                resolver
                    .resolve(&request.query, request.transaction_id)
                    .await
            }
            None => None,
        }
    } else {
        None
    }
    .unwrap_or_else(|| servfail_dns_packet(&request));

    ExitDnsResponse::new(request.nonce, request.transaction_id, response)
        .expect("resolver response must preserve the DNS transaction")
}

fn servfail_exit_dns_response(request: &ExitDnsRequest) -> ExitDnsResponse {
    ExitDnsResponse::new(
        request.nonce,
        request.transaction_id,
        servfail_dns_packet(request),
    )
    .expect("SERVFAIL must preserve the DNS transaction")
}

fn servfail_dns_packet(request: &ExitDnsRequest) -> Vec<u8> {
    build_exit_dns_servfail_response(&request.query)
        .expect("a validated DNS query always produces SERVFAIL")
}

#[cfg(any(target_os = "linux", test))]
fn initial_nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        | 1
}

#[cfg(test)]
#[path = "exit_dns_runtime_tests.rs"]
mod tests;
