//! Own discovery/indexer threads until their final persistence has completed.
use std::io;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct IndexerThreads(Mutex<State>);

#[derive(Default)]
struct State {
    stopping: bool,
    handles: Vec<JoinHandle<()>>,
}

impl IndexerThreads {
    pub(crate) fn spawn(
        &self,
        name: String,
        work: impl FnOnce() + Send + 'static,
    ) -> io::Result<()> {
        let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
        if state.stopping {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Indexer shutdown",
            ));
        }
        // Retired hotplug workers must not accumulate for the service's lifetime.
        state.handles.retain(|handle| !handle.is_finished());
        state
            .handles
            .push(thread::Builder::new().name(name).spawn(work)?);
        Ok(())
    }

    /// Called once by the service coordinator after signalling global shutdown.
    /// Closing admission under the spawn lock also covers discovery/spawn races.
    pub(crate) fn shutdown(&self, mut report_pending: impl FnMut()) {
        let handles = {
            let mut state = self.0.lock().unwrap_or_else(|error| error.into_inner());
            state.stopping = true;
            std::mem::take(&mut state.handles)
        };
        report_pending();
        while handles.iter().any(|handle| !handle.is_finished()) {
            thread::sleep(Duration::from_secs(1));
            report_pending();
        }
        for handle in handles {
            if handle.join().is_err() {
                eprintln!("[SERVICE] Indexer/discovery worker panicked before shutdown completed");
            }
        }
    }
}

#[cfg(test)]
mod tests;
