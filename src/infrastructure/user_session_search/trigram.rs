//! PERF-06: trigram inverted index for the user-session search index.
//!
//! The per-keystroke predicate for session volumes is
//! `name_lower.contains(token)` over every indexed item. For volumes up to
//! [`TRIGRAM_INDEX_MAX_ITEMS`] items, char-trigram posting lists narrow the
//! candidate set before the exact predicate runs. Trigrams are a necessary
//! but not sufficient condition of `contains`, so the exact verification
//! keeps the match set identical to the previous linear scan — only the
//! number of candidates inspected changes.
//!
//! Volumes above the threshold get no index and keep the linear scan (the
//! pre-PERF-06 behavior), which caps the index's worst-case memory.

use std::collections::HashMap;

/// Volumes above this item count get no trigram index.
///
/// Posting lists hold one `u32` per (item, distinct-position trigram); with
/// long encrypted names (~40 chars) 150k items cost roughly 20-25 MB. Above
/// the cap the linear scan is kept — documented as the extreme-size behavior.
pub(super) const TRIGRAM_INDEX_MAX_ITEMS: usize = 150_000;

/// Minimum chars a token needs to contribute trigram candidates. Shorter
/// tokens cannot narrow and force a fallback (handled by the caller).
pub(super) const MIN_TRIGRAM_TOKEN_CHARS: usize = 3;

/// Char-based trigram of a lowercased name. `[char; 3]` hashes directly and
/// avoids multibyte UTF-8 byte-boundary pitfalls.
type Trigram = [char; 3];

pub(super) struct TrigramIndex {
    /// Trigram → item indices, kept sorted ascending and duplicate-free.
    postings: HashMap<Trigram, Vec<u32>>,
}

impl TrigramIndex {
    /// Builds the index over lowercased names, parallel to the items vector.
    pub fn build<'a, I>(name_lowers: I) -> Self
    where
        I: Iterator<Item = &'a str>,
    {
        let mut postings: HashMap<Trigram, Vec<u32>> = HashMap::new();
        for (idx, name_lower) in name_lowers.enumerate() {
            let idx32 = idx as u32;
            for_each_trigram(name_lower, |trigram| {
                let list = postings.entry(trigram).or_default();
                // A name can contain the same trigram more than once
                // ("aaaa" -> "aaa" twice); indices are pushed in ascending
                // order, so a same-index duplicate is always the last entry.
                if list.last() != Some(&idx32) {
                    list.push(idx32);
                }
            });
        }
        Self { postings }
    }

    /// Incremental insertion for watcher upserts (new or renamed items).
    ///
    /// Keeps the index sound — a superset of true candidates: stale trigrams
    /// of removed/renamed names are never removed, but the exact predicate
    /// filters them out at query time. Duplicate indices are prevented by the
    /// binary-search check, preserving sorted, unique posting lists.
    pub fn insert_name(&mut self, idx: usize, name_lower: &str) {
        let idx32 = idx as u32;
        for_each_trigram(name_lower, |trigram| {
            let list = self.postings.entry(trigram).or_default();
            if let Err(pos) = list.binary_search(&idx32) {
                list.insert(pos, idx32);
            }
        });
    }

    /// Sorted candidate item indices that *may* contain `token`.
    ///
    /// Returns `None` when the token is too short to produce trigrams (the
    /// caller must fall back to the linear scan), or `Some(empty)` when the
    /// token provably matches nothing (one of its trigrams is absent from
    /// every indexed name).
    pub fn candidates_for_token(&self, token: &str) -> Option<Vec<u32>> {
        let trigrams = collect_trigrams(token);
        if trigrams.is_empty() {
            return None;
        }

        // If any trigram of the token has no postings, no item can contain
        // the token (contains(token) implies all its trigrams are present).
        let mut lists: Vec<&Vec<u32>> = Vec::with_capacity(trigrams.len());
        for trigram in &trigrams {
            match self.postings.get(trigram) {
                Some(list) => lists.push(list),
                None => return Some(Vec::new()),
            }
        }

        // Start from the rarest trigram and filter by the remaining ones.
        lists.sort_by_key(|list| list.len());
        let mut candidates: Vec<u32> = lists[0].clone();
        for list in &lists[1..] {
            candidates.retain(|idx| list.binary_search(idx).is_ok());
            if candidates.is_empty() {
                break;
            }
        }
        // Defensive: postings are duplicate-free at the source, but keep the
        // candidate set strictly unique (list stays sorted, so dedup is
        // linear and exact).
        candidates.dedup();
        Some(candidates)
    }
}

/// Distinct char trigrams of `s` (order preserved, duplicates removed).
/// Empty when `s` has fewer than [`MIN_TRIGRAM_TOKEN_CHARS`] chars.
fn collect_trigrams(s: &str) -> Vec<Trigram> {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < MIN_TRIGRAM_TOKEN_CHARS {
        return Vec::new();
    }
    let mut trigrams: Vec<Trigram> = chars.windows(3).map(|w| [w[0], w[1], w[2]]).collect();
    trigrams.sort_unstable();
    trigrams.dedup();
    trigrams
}

/// Invokes `f` for each trigram occurrence in `s` (duplicates included; the
/// index insert path deduplicates by index).
fn for_each_trigram(s: &str, mut f: impl FnMut(Trigram)) {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < MIN_TRIGRAM_TOKEN_CHARS {
        return;
    }
    for window in chars.windows(3) {
        f([window[0], window[1], window[2]]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_tokens_produce_no_trigrams() {
        assert!(collect_trigrams("ab").is_empty());
        assert!(collect_trigrams("").is_empty());
        assert_eq!(collect_trigrams("abc").len(), 1);
    }

    #[test]
    fn repeated_trigrams_are_deduplicated_per_token() {
        // "aaaa" yields the "aaa" window twice; the token contributes once.
        assert_eq!(collect_trigrams("aaaa"), vec![['a', 'a', 'a']]);
    }

    #[test]
    fn candidates_are_superset_of_true_matches() {
        let names = ["report 2024.txt", "annual-report.pdf", "résumé.txt", "ab"];
        let index = TrigramIndex::build(names.iter().copied());

        let candidates = index.candidates_for_token("report").unwrap();
        // Both items containing "report" must be present (superset property).
        assert!(candidates.contains(&0));
        assert!(candidates.contains(&1));

        // Unicode names participate in trigram narrowing.
        let candidates = index.candidates_for_token("résumé").unwrap();
        assert!(candidates.contains(&2));

        // A token with an absent trigram yields an empty (not None) set.
        let candidates = index.candidates_for_token("zebra").unwrap();
        assert!(candidates.is_empty());

        // Tokens shorter than 3 chars cannot narrow.
        assert!(index.candidates_for_token("ab").is_none());
    }

    #[test]
    fn insert_name_keeps_postings_sorted_and_unique() {
        let names = ["alpha.txt", "beta.txt"];
        let mut index = TrigramIndex::build(names.iter().copied());

        // Simulate an upsert re-inserting an existing index.
        index.insert_name(0, "alpha.txt");
        // Simulate a new item appended after the scan.
        index.insert_name(2, "alphabet.txt");

        let candidates = index.candidates_for_token("alpha").unwrap();
        assert_eq!(candidates, vec![0, 2]);

        // Repeated insertion must not duplicate indices.
        index.insert_name(2, "alphabet.txt");
        let candidates = index.candidates_for_token("alphabet").unwrap();
        assert_eq!(candidates, vec![2]);
    }
}
