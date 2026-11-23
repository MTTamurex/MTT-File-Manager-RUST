use super::*;

const IDLE_GRACE: Duration = Duration::from_secs(2);
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const MAX_CAUSE_KEYS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct WindowState {
    pub visible: bool,
    pub focused: bool,
    pub minimized: bool,
}

pub(super) struct Report {
    pub elapsed: Duration,
    pub ui_passes: u64,
    // egui's effective previous-pass deadline, after predicted frame time
    // subtraction; this is not proof of an explicit request_repaint() call.
    pub zero_previous_delay_passes: u64,
    pub logic_ticks: u64,
    pub causes: Vec<(&'static str, u32, u64)>,
    pub causes_overflow: u64,
}

#[derive(Default)]
pub(super) struct Recorder {
    session: Option<u64>,
    state: Option<WindowState>,
    last_input: Option<Instant>,
    sample_started: Option<Instant>,
    last_pass: Option<u64>,
    ui_passes: u64,
    zero_previous_delay_passes: u64,
    logic_ticks: u64,
    causes: HashMap<(&'static str, u32), u64>,
    causes_overflow: u64,
}

impl Recorder {
    pub(super) fn record(
        &mut self,
        session: u64,
        now: Instant,
        state: WindowState,
        input: bool,
        pass: Option<u64>,
        zero_previous_delay: bool,
        causes: Vec<egui::RepaintCause>,
    ) -> Option<Report> {
        if self.session != Some(session) || self.state != Some(state) {
            *self = Self {
                session: Some(session),
                state: Some(state),
                ..Default::default()
            };
        }
        if input {
            self.last_input = Some(now);
            self.sample_started = None;
            self.ui_passes = 0;
            self.zero_previous_delay_passes = 0;
            self.logic_ticks = 0;
            self.causes.clear();
            self.causes_overflow = 0;
            self.last_pass = None;
            return None;
        }
        if now.saturating_duration_since(*self.last_input.get_or_insert(now)) < IDLE_GRACE {
            return None;
        }
        let started = match self.sample_started {
            Some(started) => started,
            None => {
                self.sample_started = Some(now);
                return None;
            }
        };
        if let Some(pass) = pass {
            if self.last_pass != Some(pass) {
                self.last_pass = Some(pass);
                self.ui_passes = self.ui_passes.saturating_add(1);
                if zero_previous_delay {
                    self.zero_previous_delay_passes =
                        self.zero_previous_delay_passes.saturating_add(1);
                }
                for cause in causes {
                    let key = (classify_source(cause.file), cause.line);
                    if self.causes.len() < MAX_CAUSE_KEYS || self.causes.contains_key(&key) {
                        let count = self.causes.entry(key).or_default();
                        *count = count.saturating_add(1);
                    } else {
                        self.causes_overflow = self.causes_overflow.saturating_add(1);
                    }
                }
            }
        } else {
            self.logic_ticks = self.logic_ticks.saturating_add(1);
        }
        let elapsed = now.saturating_duration_since(started);
        if elapsed < SAMPLE_INTERVAL {
            return None;
        }
        // logic() precedes ui() in a visible pass. Finish that cycle before
        // publishing, rather than assigning its UI work to the next interval.
        // Hidden windows must still report without requiring a UI callback.
        if pass.is_none() && state.visible && !state.minimized {
            return None;
        }
        let report = Report {
            elapsed,
            ui_passes: self.ui_passes,
            zero_previous_delay_passes: self.zero_previous_delay_passes,
            logic_ticks: self.logic_ticks,
            causes: self
                .causes
                .iter()
                .map(|(&(source, line), &count)| (source, line, count))
                .collect(),
            causes_overflow: self.causes_overflow,
        };
        self.sample_started = Some(now);
        self.ui_passes = 0;
        self.zero_previous_delay_passes = 0;
        self.logic_ticks = 0;
        self.causes.clear();
        self.causes_overflow = 0;
        Some(report)
    }
}

#[cfg(test)]
mod tests;
