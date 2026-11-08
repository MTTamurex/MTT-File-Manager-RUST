//! Stable dispersion within the middle half, using the existing cache-key hash.
use std::path::Path;

pub(crate) const MAX_CANDIDATES: usize = 3;

pub(crate) fn seed(path: &Path) -> u64 {
    let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
    u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
}

pub(crate) fn duration(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

pub(crate) fn candidates(duration: Option<f64>, seed: u64) -> [f64; MAX_CANDIDATES] {
    let Some(duration) = self::duration(duration) else {
        // No percentage seek without a known duration. Later probes are only
        // attempted if the initial frame is unsuitable, and may hit EOF.
        return [0.0, 0.5, 1.0];
    };
    let fraction = (seed >> 11) as f64 / (1u64 << 53) as f64;
    std::array::from_fn(|index| {
        let dispersed = (fraction + index as f64 / MAX_CANDIDATES as f64).fract();
        duration * (0.25 + dispersed * 0.5)
    })
}

pub(crate) fn outside_middle(actual: Option<f64>, duration: Option<f64>) -> bool {
    match (actual, self::duration(duration)) {
        (Some(actual), Some(duration)) => actual < duration * 0.25 || actual > duration * 0.75,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_are_stable_bounded_and_separated() {
        for value in [0, 1, 12345, u64::MAX] {
            let positions = candidates(Some(1200.0), value);
            assert_eq!(positions, candidates(Some(1200.0), value));
            assert!(positions.iter().all(|&p| (300.0..=900.0).contains(&p)));
            for i in 0..MAX_CANDIDATES {
                for j in i + 1..MAX_CANDIDATES {
                    assert!((positions[i] - positions[j]).abs() >= 199.99);
                }
            }
        }
    }

    #[test]
    fn cache_keys_disperse_similar_episode_names() {
        let first = Path::new(r"C:\Videos\episode01.mp4");
        let second = Path::new(r"C:\Videos\episode02.mp4");
        assert_eq!(seed(first), seed(first));
        assert_ne!(
            candidates(Some(1200.0), seed(first)),
            candidates(Some(1200.0), seed(second))
        );
    }

    #[test]
    fn short_and_unknown_durations_do_not_generate_out_of_range_percentages() {
        for duration in [0.001, 0.2, 1.0] {
            assert!(candidates(Some(duration), 7)
                .iter()
                .all(|&p| p >= 0.0 && p < duration));
        }
        for invalid in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            assert_eq!(candidates(invalid, 7), [0.0, 0.5, 1.0]);
        }
        assert!(outside_middle(Some(0.0), Some(3.0)));
        assert!(!outside_middle(Some(1.0), Some(3.0)));
    }
}
