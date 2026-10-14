pub(super) fn normalize_volume_id(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

pub(super) fn volume_ids_match(left: &str, right: &str) -> bool {
    let left = normalize_volume_id(left);
    let right = normalize_volume_id(right);
    if left == right {
        return true;
    }

    let volume_token = |value: &str| {
        let lower = value.to_ascii_lowercase();
        let start = lower.find("volume{")?;
        let rest = &value[start..];
        let end = rest.find('}')? + 1;
        Some(rest[..end].to_ascii_lowercase())
    };
    volume_token(&left).is_some() && volume_token(&left) == volume_token(&right)
}
