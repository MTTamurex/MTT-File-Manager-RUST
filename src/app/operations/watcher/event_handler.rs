use crate::app::state::TimestampedNotifyEvent;

pub(super) fn handler(
    tx: crossbeam_channel::Sender<TimestampedNotifyEvent>,
    overflow_rx: crossbeam_channel::Receiver<TimestampedNotifyEvent>,
    ctx: eframe::egui::Context,
) -> impl notify::EventHandler {
    move |res: notify::Result<notify::Event>| {
        match &res {
            Ok(event) => log::trace!(
                "[NOTIFY-WATCHER] Event received: kind={:?}, paths={:?}",
                event.kind,
                event.paths
            ),
            Err(error) => log::error!("[NOTIFY-WATCHER] Event error: {}", error),
        }
        let event = TimestampedNotifyEvent {
            received_at: std::time::Instant::now(),
            result: res,
            overflow: false,
        };
        match tx.try_send(event) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(dropped)) => {
                // Drop the oldest queued event and coalesce the burst into one
                // overflow marker so the UI schedules a debounced reload.
                let _ = overflow_rx.try_recv();
                let paths = match dropped.result {
                    Ok(event) => event.paths,
                    Err(error) => error.paths,
                };
                let marker = notify::Event {
                    kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
                    paths,
                    attrs: Default::default(),
                };
                let mut overflow_event = TimestampedNotifyEvent {
                    received_at: std::time::Instant::now(),
                    result: Ok(marker),
                    overflow: true,
                };
                loop {
                    match tx.try_send(overflow_event) {
                        Ok(()) => break,
                        Err(crossbeam_channel::TrySendError::Full(returned)) => {
                            let _ = overflow_rx.try_recv();
                            overflow_event = returned;
                        }
                        Err(crossbeam_channel::TrySendError::Disconnected(_)) => break,
                    }
                }
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
        }
        ctx.request_repaint();
    }
}
