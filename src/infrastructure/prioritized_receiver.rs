use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::time::Duration;

pub(crate) enum PrioritizedReceive<T> {
    Request(T),
    Timeout,
    Disconnected,
}

pub(crate) fn receive_prioritized<T>(
    work_rx: &Receiver<T>,
    control_rx: &Receiver<T>,
    deferred_work: &mut Option<T>,
) -> PrioritizedReceive<T> {
    match control_rx.try_recv() {
        Ok(request) => return PrioritizedReceive::Request(request),
        Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
    }
    if let Some(request) = deferred_work.take() {
        return PrioritizedReceive::Request(request);
    }

    match work_rx.recv_timeout(Duration::from_millis(25)) {
        Ok(request) => match control_rx.try_recv() {
            Ok(control_request) => {
                *deferred_work = Some(request);
                PrioritizedReceive::Request(control_request)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                PrioritizedReceive::Request(request)
            }
        },
        Err(RecvTimeoutError::Timeout) => match control_rx.try_recv() {
            Ok(request) => PrioritizedReceive::Request(request),
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => PrioritizedReceive::Timeout,
        },
        Err(RecvTimeoutError::Disconnected) => {
            match control_rx.recv_timeout(Duration::from_millis(25)) {
                Ok(request) => PrioritizedReceive::Request(request),
                Err(RecvTimeoutError::Timeout) => PrioritizedReceive::Timeout,
                Err(RecvTimeoutError::Disconnected) => PrioritizedReceive::Disconnected,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{receive_prioritized, PrioritizedReceive};
    use std::sync::mpsc;

    #[test]
    fn control_overtakes_work_received_while_waiting() {
        let (work_tx, work_rx) = mpsc::sync_channel(1);
        let (control_tx, control_rx) = mpsc::sync_channel(1);
        control_tx.send("cancel").unwrap();
        work_tx.send("extract").unwrap();
        let mut deferred = None;

        assert!(matches!(
            receive_prioritized(&work_rx, &control_rx, &mut deferred),
            PrioritizedReceive::Request("cancel")
        ));
        assert!(matches!(
            receive_prioritized(&work_rx, &control_rx, &mut deferred),
            PrioritizedReceive::Request("extract")
        ));
    }
}
