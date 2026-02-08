// Downscale filter applied only in docked mode (preview in sidebar)
pub const DOCKED_DOWNSCALE_FILTER: &str =
    "scale=w='min(iw,854)':h='min(ih,480)':force_original_aspect_ratio=decrease";
pub const DOCKED_DOWNSCALE_MARKER: &str = "min(ih,480)";
// FPS limit filter applied only in docked mode (preview in sidebar)
pub const DOCKED_FPS_FILTER: &str = "fps=fps=30";
pub const DOCKED_FPS_MARKER: &str = "fps=fps=30";
pub const DEINTERLACE_FILTER: &str = "bwdif=mode=auto:parity=auto:deint=all";
pub const DEINTERLACE_MARKER: &str = "bwdif=";
pub const AUDIO_NORMALIZER_FILTER: &str = "dynaudnorm=f=75";
pub const AUDIO_NORMALIZER_MARKER: &str = "dynaudnorm";

/// Append a video filter to the current filter chain
pub fn append_vf_filter(current_vf: &str, filter: &str) -> String {
    if current_vf.trim().is_empty() {
        filter.to_string()
    } else {
        format!("{},{}", current_vf, filter)
    }
}

/// Remove a video filter from the current filter chain by marker
pub fn remove_vf_filter(current_vf: &str, marker: &str) -> String {
    let mut parts: Vec<&str> = current_vf
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    parts.retain(|part| !part.contains(marker));
    parts.join(",")
}

/// Append an audio filter to the current filter chain
pub fn append_af_filter(current_af: &str, filter: &str) -> String {
    if current_af.trim().is_empty() {
        filter.to_string()
    } else {
        format!("{},{}", current_af, filter)
    }
}

/// Remove an audio filter from the current filter chain by marker
pub fn remove_af_filter(current_af: &str, marker: &str) -> String {
    let mut parts: Vec<&str> = current_af
        .split(',')
        .map(|part| part.trim())
        .filter(|part| !part.is_empty())
        .collect();
    parts.retain(|part| !part.contains(marker));
    parts.join(",")
}
