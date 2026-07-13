#[derive(Debug, Clone)]
pub(crate) struct ReceivedJoinApprovalAck {
    pub(crate) source_peer: String,
    pub(crate) ack:
        nostr_vpn_core::identity_bridge::NostrIdentityDeviceApprovalAppliedAck,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub(crate) struct DirectJoinApprovalAckRuntime {
    received: mpsc::Receiver<ReceivedJoinApprovalAck>,
    task: tokio::task::JoinHandle<()>,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl DirectJoinApprovalAckRuntime {
    pub(crate) async fn start(endpoint: Arc<FipsEndpoint>) -> Result<Self> {
        let receiver = endpoint
            .register_service_receiver(NOSTR_JOIN_PUBSUB_FIPS_SERVICE_PORT)
            .await
            .context("failed to register direct join approval ack service")?;
        let (received_tx, received) = mpsc::channel(32);
        let task = tokio::spawn(async move {
            let mut datagrams = Vec::with_capacity(16);
            while receiver.recv_batch_into(&mut datagrams, 16).await.is_some() {
                for datagram in &datagrams {
                    let inbound =
                        nostr_vpn_core::join_pubsub::NostrJoinFipsPubsubDatagram {
                            source_port: datagram.source_port,
                            destination_port: datagram.destination_port,
                            payload: datagram.data.as_ref().to_vec(),
                        };
                    let Ok(ack) =
                        nostr_vpn_core::join_pubsub::parse_approval_applied_ack_datagram(&inbound)
                    else {
                        continue;
                    };
                    if received_tx
                        .send(ReceivedJoinApprovalAck {
                            source_peer: datagram.source_peer.npub(),
                            ack,
                        })
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
            }
        });
        Ok(Self { received, task })
    }

    pub(crate) fn drain(&mut self) -> Vec<ReceivedJoinApprovalAck> {
        let mut received = Vec::new();
        while let Ok(ack) = self.received.try_recv() {
            received.push(ack);
        }
        received
    }

    pub(crate) async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}
