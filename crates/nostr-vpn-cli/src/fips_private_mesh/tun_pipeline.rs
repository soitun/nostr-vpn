#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn drain_event_batch(
    event_rx: &mut mpsc::Receiver<FipsPrivateMeshEvent>,
    limit: usize,
) -> Vec<FipsPrivateMeshEvent> {
    let mut events = Vec::new();
    for _ in 0..limit {
        let Ok(event) = event_rx.try_recv() else {
            break;
        };
        events.push(event);
    }
    events
}
