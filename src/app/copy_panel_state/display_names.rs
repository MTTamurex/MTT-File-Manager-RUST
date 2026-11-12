//! Display-only sampling. Never changes operation counters or worker scheduling.
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct DisplayNames {
    sampled: Option<Instant>,
    paths: Vec<PathBuf>,
}

impl DisplayNames {
    pub fn snapshot(&mut self, paths: Vec<PathBuf>, now: Instant) -> Vec<PathBuf> {
        if self
            .sampled
            .is_none_or(|at| now.saturating_duration_since(at) >= Duration::from_millis(300))
        {
            self.paths = paths;
            self.sampled = Some(now);
        }
        self.paths.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_change_at_most_every_three_hundred_milliseconds() {
        let mut names = DisplayNames::default();
        let now = Instant::now();
        let first = vec![PathBuf::from("source/first.jpg")];
        assert_eq!(names.snapshot(first.clone(), now), first);
        assert_eq!(
            names.snapshot(vec![], now + Duration::from_millis(299)),
            first
        );
        let next = vec![PathBuf::from("source/next.jpg")];
        assert_eq!(
            names.snapshot(next.clone(), now + Duration::from_millis(300)),
            next
        );
    }
}
