use super::state::{FileLoadGate, MpvState};
use std::sync::RwLock;

/// Poll duration before publishing positions, retrying every tick until ready.
pub(super) fn refresh_duration(
    state: &RwLock<MpvState>,
    gate: &FileLoadGate,
    generation: u64,
    periodic_poll: bool,
    read_duration: impl FnOnce() -> Option<f64>,
) -> bool {
    let cached_duration = state.read().map(|state| state.duration).unwrap_or(0.0);
    if !periodic_poll && cached_duration.is_finite() && cached_duration > 0.0 {
        return false;
    }

    // Do not hold the state lock across an MPV call.
    let Some(duration) = read_duration().filter(|value| value.is_finite() && *value > 0.0) else {
        return false;
    };
    if let Ok(mut state) = state.write() {
        if gate.result_is_current(generation)
            && (!state.duration.is_finite()
                || state.duration <= 0.0
                || (state.duration - duration).abs() > 0.01)
        {
            state.duration = duration;
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_duration_is_retried_without_waiting_for_the_periodic_tick() {
        let state = RwLock::new(MpvState::default());
        let gate = FileLoadGate::default();
        assert!(!refresh_duration(&state, &gate, 0, false, || None));
        assert!(refresh_duration(&state, &gate, 0, false, || Some(120.0)));
        assert_eq!(state.read().unwrap().duration, 120.0);
        assert!(!refresh_duration(&state, &gate, 0, false, || {
            panic!("known duration should keep periodic polling")
        }));
        assert!(refresh_duration(&state, &gate, 0, true, || Some(121.0)));
    }

    #[test]
    fn invalid_duration_does_not_replace_a_valid_timeline() {
        let state = RwLock::new(MpvState {
            duration: 120.0,
            ..Default::default()
        });
        let gate = FileLoadGate::default();
        for duration in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(!refresh_duration(&state, &gate, 0, true, || Some(duration)));
            assert_eq!(state.read().unwrap().duration, 120.0);
        }
    }

    #[test]
    fn load_change_during_poll_cannot_publish_a_stale_duration() {
        let state = RwLock::new(MpvState::default());
        let gate = FileLoadGate::default();
        assert!(!refresh_duration(&state, &gate, 0, false, || {
            gate.begin_load("other.mp4".into());
            Some(120.0)
        }));
        assert_eq!(state.read().unwrap().duration, 0.0);
    }
}
