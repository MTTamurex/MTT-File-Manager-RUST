use std::sync::{Arc, Mutex};
use std::time::Instant;
use windows::core::HRESULT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    AsyncMode(bool),
    AsyncModeQueried,
    Started,
    Ended {
        result: HRESULT,
        effects: u32,
    },
    Feedback {
        format: &'static str,
        effect: Option<u32>,
        accepted: bool,
    },
    DropReturned,
}

struct State {
    started: Instant,
    signals: Vec<Signal>,
}

#[derive(Clone)]
pub struct Observation(Arc<Mutex<State>>);

impl Observation {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            started: Instant::now(),
            signals: Vec::new(),
        })))
    }

    pub fn record(&self, signal: Signal) {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        // Only protocol names, numerical effects and elapsed time are logged.
        // Do not log file paths or arbitrary data-object payloads.
        println!("[Observe +{:?}] {signal:?}", state.started.elapsed());
        state.signals.push(signal);
    }

    pub fn summary(&self) {
        let state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let starts = state
            .signals
            .iter()
            .filter(|s| **s == Signal::Started)
            .count();
        let ends = state
            .signals
            .iter()
            .filter(|s| matches!(s, Signal::Ended { .. }))
            .count();
        let feedback = state
            .signals
            .iter()
            .filter(|s| matches!(s, Signal::Feedback { .. }))
            .count();
        println!(
            "[Observe summary] StartOperation={starts}, EndOperation={ends}, feedback={feedback}"
        );
        println!(
            "Protocol signals alone are NOT proof that an external transfer finished successfully."
        );
    }

    #[cfg(test)]
    pub fn signals(&self) -> Vec<Signal> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .signals
            .clone()
    }
}
