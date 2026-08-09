use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

#[derive(Clone)]
pub struct PendingSeekState {
    pub target_time: f64,
    pub requested_at: Instant,
}

impl PendingSeekState {
    pub fn new(target_time: f64) -> Self {
        Self {
            target_time,
            requested_at: Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct PendingFileLoad {
    pub generation: u64,
    pub expected_path: String,
    pub requested_at: Instant,
}

#[derive(Default)]
struct FileLoadGateState {
    pending: Option<PendingFileLoad>,
    registrations: VecDeque<PendingFileLoad>,
    loaded_generation: Option<u64>,
}

/// Serializes file transitions while exposing lock-free publication checks.
pub struct FileLoadGate {
    generation: AtomicU64,
    loading_generation: AtomicU64,
    command_pending: AtomicBool,
    parked: AtomicBool,
    state: Mutex<FileLoadGateState>,
}

impl Default for FileLoadGate {
    fn default() -> Self {
        Self {
            generation: AtomicU64::new(0),
            loading_generation: AtomicU64::new(0),
            command_pending: AtomicBool::new(false),
            parked: AtomicBool::new(false),
            state: Mutex::new(FileLoadGateState::default()),
        }
    }
}

impl FileLoadGate {
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub fn loading_generation(&self) -> u64 {
        self.loading_generation.load(Ordering::Acquire)
    }

    pub fn is_command_pending(&self) -> bool {
        self.command_pending.load(Ordering::Acquire)
    }

    pub fn is_parked(&self) -> bool {
        self.parked.load(Ordering::Acquire)
    }

    pub fn result_is_current(&self, observed_generation: u64) -> bool {
        !self.is_command_pending() && !self.is_parked() && self.generation() == observed_generation
    }

    fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn begin_load(&self, expected_path: String) -> u64 {
        let mut state = self.state.lock().expect("file load gate lock");
        self.command_pending.store(true, Ordering::Release);
        let generation = self.next_generation();
        let pending = PendingFileLoad {
            generation,
            expected_path,
            requested_at: Instant::now(),
        };
        self.loading_generation.store(generation, Ordering::Release);
        self.parked.store(true, Ordering::Release);
        state.pending = Some(pending.clone());
        state.registrations.push_back(pending);
        state.loaded_generation = None;
        generation
    }

    pub fn finish_load_command(&self, generation: u64, succeeded: bool) {
        let mut state = self.state.lock().expect("file load gate lock");
        if self.generation() != generation {
            return;
        }

        if !succeeded {
            state
                .registrations
                .retain(|registration| registration.generation != generation);
            state.pending = None;
            state.loaded_generation = None;
            self.loading_generation.store(0, Ordering::Release);
            self.parked.store(false, Ordering::Release);
        } else if state.loaded_generation == Some(generation) {
            state.pending = None;
            state.loaded_generation = None;
            self.loading_generation.store(0, Ordering::Release);
            self.parked.store(false, Ordering::Release);
        }
        self.command_pending.store(false, Ordering::Release);
    }

    pub fn take_start_file_registration(&self) -> Option<PendingFileLoad> {
        self.state
            .lock()
            .expect("file load gate lock")
            .registrations
            .pop_front()
    }

    pub fn note_file_loaded(&self, generation: u64) -> bool {
        let mut state = self.state.lock().expect("file load gate lock");
        if self.generation() != generation || self.loading_generation() != generation {
            return false;
        }

        state
            .registrations
            .retain(|registration| registration.generation != generation);
        state.loaded_generation = Some(generation);
        if self.is_command_pending() {
            return false;
        }

        state.pending = None;
        state.loaded_generation = None;
        self.loading_generation.store(0, Ordering::Release);
        self.parked.store(false, Ordering::Release);
        true
    }

    pub fn cancel_load(&self, generation: u64) -> bool {
        let mut state = self.state.lock().expect("file load gate lock");
        if self.generation() != generation || self.loading_generation() != generation {
            return false;
        }

        state
            .registrations
            .retain(|registration| registration.generation != generation);
        state.pending = None;
        state.loaded_generation = None;
        self.loading_generation.store(0, Ordering::Release);
        self.parked.store(false, Ordering::Release);
        self.command_pending.store(false, Ordering::Release);
        true
    }

    pub fn cancel_waiting_transition(&self) {
        let mut state = self.state.lock().expect("file load gate lock");
        state.pending = None;
        state.loaded_generation = None;
        self.loading_generation.store(0, Ordering::Release);
        self.parked.store(false, Ordering::Release);
        self.command_pending.store(false, Ordering::Release);
    }

    pub fn pending_load(&self) -> Option<PendingFileLoad> {
        self.state
            .lock()
            .expect("file load gate lock")
            .pending
            .clone()
    }

    pub fn park(&self) -> bool {
        let mut state = self.state.lock().expect("file load gate lock");
        let was_parked = self.parked.swap(true, Ordering::AcqRel);
        let should_invalidate = !was_parked || self.loading_generation() != 0;
        self.command_pending.store(true, Ordering::Release);
        if should_invalidate {
            self.next_generation();
        }
        self.loading_generation.store(0, Ordering::Release);
        state.pending = None;
        state.loaded_generation = None;
        should_invalidate
    }

    pub fn finish_park(&self) {
        self.command_pending.store(false, Ordering::Release);
    }

    pub fn prepare_retarget(&self) {
        let mut state = self.state.lock().expect("file load gate lock");
        self.command_pending.store(true, Ordering::Release);
        let generation = self.next_generation();
        self.loading_generation.store(generation, Ordering::Release);
        self.parked.store(true, Ordering::Release);
        state.pending = None;
        state.loaded_generation = None;
    }

    pub fn finish_retarget(&self) {
        self.command_pending.store(false, Ordering::Release);
    }

    pub fn shutdown(&self) {
        let mut state = self.state.lock().expect("file load gate lock");
        self.command_pending.store(true, Ordering::Release);
        self.parked.store(true, Ordering::Release);
        self.next_generation();
        self.loading_generation.store(0, Ordering::Release);
        state.pending = None;
        state.registrations.clear();
        state.loaded_generation = None;
    }
}

/// Track information for audio/subtitles.
#[derive(Clone, Debug, Default)]
pub struct TrackInfo {
    pub id: i64,
    pub track_type: String, // "audio", "video", "sub"
    pub title: Option<String>,
    pub lang: Option<String>,
    pub selected: bool,
}

/// Shared state for MPV playback.
#[derive(Clone, Default)]
pub struct MpvState {
    pub is_playing: bool,
    pub current_time: f64,
    pub duration: f64,
    pub volume: f32,
    pub is_muted: bool,
    pub audio_tracks: Vec<TrackInfo>,
    pub subtitle_tracks: Vec<TrackInfo>,
    // PERF: fields polled by background event loop to avoid FFI on render thread
    pub fullscreen: bool,
    pub video_aspect: Option<f64>,
    pub interlaced: Option<bool>,
    pub tracks_ready: bool,
}

#[cfg(test)]
mod file_load_gate_tests {
    use super::FileLoadGate;

    #[test]
    fn file_loaded_releases_only_the_current_generation() {
        let gate = FileLoadGate::default();
        let generation = gate.begin_load("B.mp4".into());
        let registration = gate
            .take_start_file_registration()
            .expect("registered load");
        assert_eq!(registration.generation, generation);

        gate.finish_load_command(generation, true);
        assert!(gate.is_parked());
        assert_eq!(gate.loading_generation(), generation);
        assert!(!gate.note_file_loaded(generation + 1));
        assert!(gate.is_parked());

        assert!(gate.note_file_loaded(generation));
        assert!(!gate.is_parked());
        assert_eq!(gate.loading_generation(), 0);
        assert!(gate.result_is_current(generation));
    }

    #[test]
    fn file_loaded_waits_for_the_load_command_to_finish() {
        let gate = FileLoadGate::default();
        let generation = gate.begin_load("B.mp4".into());

        assert!(!gate.note_file_loaded(generation));
        assert!(gate.is_parked());
        gate.finish_load_command(generation, true);

        assert!(!gate.is_parked());
        assert_eq!(gate.loading_generation(), 0);
    }

    #[test]
    fn stale_file_loaded_event_cannot_release_a_newer_load() {
        let gate = FileLoadGate::default();
        let old_generation = gate.begin_load("A.mp4".into());
        gate.finish_load_command(old_generation, true);
        gate.prepare_retarget();
        gate.finish_retarget();
        let new_generation = gate.begin_load("B.mp4".into());
        gate.finish_load_command(new_generation, true);

        let old_registration = gate
            .take_start_file_registration()
            .expect("old registration");
        assert_eq!(old_registration.generation, old_generation);
        assert!(!gate.note_file_loaded(old_registration.generation));
        assert!(gate.is_parked());

        let new_registration = gate
            .take_start_file_registration()
            .expect("new registration");
        assert_eq!(new_registration.generation, new_generation);
        assert!(gate.note_file_loaded(new_registration.generation));
        assert!(!gate.is_parked());
    }

    #[test]
    fn unsafe_path_cancellation_unblocks_a_waiting_retarget() {
        let gate = FileLoadGate::default();
        gate.prepare_retarget();
        gate.finish_retarget();
        assert!(gate.is_parked());

        gate.cancel_waiting_transition();

        assert!(!gate.is_parked());
        assert!(!gate.is_command_pending());
        assert_eq!(gate.loading_generation(), 0);
    }

    #[test]
    fn failed_load_command_cancels_its_registration_and_gate() {
        let gate = FileLoadGate::default();
        let generation = gate.begin_load("B.mp4".into());
        gate.finish_load_command(generation, false);

        assert!(!gate.is_parked());
        assert!(gate.take_start_file_registration().is_none());
        assert!(gate.pending_load().is_none());
    }

    #[test]
    fn parking_an_already_parked_load_still_invalidates_it() {
        let gate = FileLoadGate::default();
        let loading_generation = gate.begin_load("B.mp4".into());
        assert!(gate.is_parked());

        assert!(gate.park());

        assert!(gate.generation() > loading_generation);
        assert_eq!(gate.loading_generation(), 0);
        assert!(gate.pending_load().is_none());
        assert!(gate.is_command_pending());
        assert!(!gate.note_file_loaded(loading_generation));

        gate.finish_park();
        assert!(gate.is_parked());
        assert!(!gate.is_command_pending());

        let parked_generation = gate.generation();
        assert!(!gate.park());
        assert_eq!(gate.generation(), parked_generation);
        gate.finish_park();
    }
}
