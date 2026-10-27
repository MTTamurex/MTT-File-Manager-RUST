//! Presentation-only shortening. Targets and original paths are never modified.
use std::collections::HashMap;

const ELLIPSIS: &str = "…";

pub(super) fn compact(value: &str, budget: i32, measure: &impl Fn(&str) -> i32) -> String {
    let value = value.replace(['\t', '\r', '\n', '\0'], " ");
    if measure(&value) <= budget {
        return value;
    }
    if let Some(root_len) = root_length(&value) {
        let (root, rest) = value.split_at(root_len);
        let rest = rest.trim_end_matches(['\\', '/']);
        let starts = component_starts(rest);
        if let Some(&last) = starts.last() {
            let separator = if root.ends_with('/') { '/' } else { '\\' };
            let prefix = format!("{root}{ELLIPSIS}{separator}");
            let leaf = &rest[last..];
            if measure(&prefix) < budget {
                if measure(&format!("{prefix}{leaf}")) > budget {
                    let leaf = middle(leaf, budget - measure(&prefix), measure);
                    let candidate = format!("{prefix}{leaf}");
                    return if measure(&candidate) <= budget {
                        candidate
                    } else {
                        middle(&candidate, budget, measure)
                    };
                }
                // Find the longest suffix of whole components that still fits.
                let mut low = 0;
                let mut high = starts.len() - 1;
                let mut best = format!("{prefix}{leaf}");
                while low <= high {
                    let mid = low + (high - low) / 2;
                    let candidate = format!("{prefix}{}", &rest[starts[mid]..]);
                    if measure(&candidate) <= budget {
                        best = candidate;
                        if mid == 0 {
                            break;
                        }
                        high = mid - 1;
                    } else {
                        low = mid + 1;
                    }
                }
                return best;
            }
        }
    }
    middle(&value, budget, measure)
}

fn root_length(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
    {
        return Some(3);
    }
    if let Some(rest) = value.strip_prefix("\\\\") {
        // Keep the server/share root for UNC paths when space permits.
        return rest.match_indices('\\').nth(1).map(|(i, _)| i + 3);
    }
    None
}

fn component_starts(value: &str) -> Vec<usize> {
    if value.is_empty() {
        return Vec::new();
    }
    std::iter::once(0)
        .chain(
            value
                .match_indices(['\\', '/'])
                .map(|(i, _)| i + 1)
                .filter(|&i| i < value.len()),
        )
        .collect()
}

fn middle(value: &str, budget: i32, measure: &impl Fn(&str) -> i32) -> String {
    if measure(value) <= budget {
        return value.to_owned();
    }
    if measure(ELLIPSIS) > budget {
        return String::new();
    }
    let boundaries: Vec<usize> = value
        .char_indices()
        .map(|(i, _)| i)
        .chain(Some(value.len()))
        .collect();
    let count = boundaries.len() - 1;
    let mut low = 0;
    let mut high = count.saturating_sub(1);
    let mut best = ELLIPSIS.to_owned();
    while low <= high {
        let keep = low + (high - low) / 2;
        let head = keep.div_ceil(2);
        let tail = keep / 2;
        let candidate = format!(
            "{}{}{}",
            &value[..boundaries[head]],
            ELLIPSIS,
            &value[boundaries[count - tail]..]
        );
        if measure(&candidate) <= budget {
            best = candidate;
            low = keep + 1;
        } else {
            if keep == 0 {
                break;
            }
            high = keep - 1;
        }
    }
    best
}

pub(super) fn siblings(
    values: &[&str],
    budget: i32,
    measure: &impl Fn(&str) -> i32,
) -> Vec<String> {
    let mut labels: Vec<_> = values
        .iter()
        .map(|value| compact(value, budget, measure))
        .collect();
    // If shortening hid the distinguishing ancestor, devote part of the label
    // to the parent context instead of letting a very long leaf consume it all.
    for group in collisions(values, &labels) {
        for index in group {
            if let Some((parent, leaf)) = values[index]
                .trim_end_matches(['\\', '/'])
                .rsplit_once(['\\', '/'])
            {
                let context = compact(parent, budget * 2 / 5, measure);
                let prefix = format!("{context} — ");
                let leaf = middle(leaf, budget - measure(&prefix), measure);
                labels[index] = compact(&format!("{prefix}{leaf}"), budget, measure);
            }
        }
    }
    // Some names differ only in the omitted characters. A bounded ordinal is
    // the last resort; duplicate tabs with the exact same path stay identical.
    if !collisions(values, &labels).is_empty() {
        let mut ordinals = HashMap::new();
        for index in 0..values.len() {
            let next = ordinals.len() + 1;
            let ordinal = *ordinals.entry(values[index]).or_insert(next);
            let suffix = format!(" [{ordinal}]");
            let label = compact(&labels[index], budget - measure(&suffix), measure);
            labels[index] = compact(&format!("{label}{suffix}"), budget, measure);
        }
    }
    labels
}

fn collisions(values: &[&str], labels: &[String]) -> Vec<Vec<usize>> {
    let mut groups: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, label) in labels.iter().enumerate() {
        groups.entry(label).or_default().push(index);
    }
    groups
        .into_values()
        .filter(|group| group.iter().any(|&i| values[i] != values[group[0]]))
        .collect()
}

#[cfg(test)]
mod tests;
