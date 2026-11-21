use crossbeam_channel::{Receiver, Sender, TrySendError};

/// Keep one pending request per preview worker; a newer selection replaces
/// queued work instead of growing an unbounded queue.
pub(super) fn send_latest<T>(
    sender: &Sender<T>,
    queue: &Receiver<T>,
    mut request: T,
) -> Result<(), T> {
    loop {
        match sender.try_send(request) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(returned)) => {
                request = returned;
                let _ = queue.try_recv();
                std::thread::yield_now();
            }
            Err(TrySendError::Disconnected(returned)) => return Err(returned),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::bounded;

    #[test]
    fn newest_selection_replaces_pending_work() {
        let (sender, queue) = bounded(1);
        send_latest(&sender, &queue, 1).unwrap();
        send_latest(&sender, &queue, 2).unwrap();
        assert_eq!(queue.try_recv(), Ok(2));
        assert!(queue.is_empty());
    }

    #[test]
    fn replacing_request_disconnects_its_reply_channel() {
        let (sender, queue) = bounded(1);
        let (old_reply, old_result) = bounded::<()>(1);
        let (new_reply, _) = bounded(1);
        send_latest(&sender, &queue, old_reply).unwrap();
        send_latest(&sender, &queue, new_reply).unwrap();
        assert!(matches!(
            old_result.try_recv(),
            Err(crossbeam_channel::TryRecvError::Disconnected)
        ));
    }
}
