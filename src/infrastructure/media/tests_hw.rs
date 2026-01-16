
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detection_runs_without_panic() {
        let caps = get_capabilities();
        println!("Detected Capabilities: {:?}", caps);
        // We can't assert values since they depend on the host machine,
        // but it shouldn't crash.
        assert!(true);
    }

    #[test]
    fn test_profile_returns_valid_codec() {
        let profile = get_optimal_profile();
        println!("Optimal Profile: {:?}", profile);
        assert!(!profile.video_codec.is_empty());
    }
}
