//! Shared filename query semantics for local and indexed search.

/// Semicolon-separated alternatives; whitespace-separated terms within each
/// alternative are ANDed. Plain terms are substrings, wildcard terms match the
/// entire filename. `*` matches any sequence and `?` one Unicode scalar value.
pub struct NameQuery {
    alternatives: Vec<Vec<Term>>,
}

struct Term {
    text: String,
    mask: Option<Vec<char>>,
}

impl NameQuery {
    pub fn uses_masks_or_alternatives(query: &str) -> bool {
        query.contains(['*', '?', ';'])
    }

    pub fn new(query: &str) -> Self {
        let alternatives = query
            .split(';')
            .map(|alternative| {
                alternative
                    .split_whitespace()
                    .map(|term| {
                        let text = term.to_lowercase();
                        let mask = text.contains(['*', '?']).then(|| text.chars().collect());
                        Term { text, mask }
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|terms| !terms.is_empty())
            .collect();
        Self { alternatives }
    }

    pub fn is_empty(&self) -> bool {
        self.alternatives.is_empty()
    }

    pub fn matches(&self, name: &str) -> bool {
        self.matches_lowercase(&name.to_lowercase())
    }

    /// The caller may reuse its cached Unicode-lowercased filename.
    pub fn matches_lowercase(&self, name: &str) -> bool {
        self.alternatives.iter().any(|terms| {
            terms.iter().all(|term| match &term.mask {
                Some(mask) => wildcard_matches(mask, name),
                None => name.contains(&term.text),
            })
        })
    }

    /// Each alternative's literal runs are necessary (but not sufficient) for
    /// matching. Trigram indices can safely narrow on these without indexing
    /// wildcard characters. Alternatives must be unioned, not intersected.
    pub fn literal_alternatives(&self) -> Vec<Vec<&str>> {
        self.alternatives
            .iter()
            .map(|terms| {
                terms
                    .iter()
                    .flat_map(|term| term.text.split(['*', '?']).filter(|run| !run.is_empty()))
                    .collect()
            })
            .collect()
    }
}

fn wildcard_matches(pattern: &[char], mut name: &str) -> bool {
    let mut position = 0;
    let mut star = None;
    let mut retry_name = name;
    while let Some(character) = name.chars().next() {
        match pattern.get(position) {
            Some('*') => {
                star = Some(position);
                position += 1;
                retry_name = name;
            }
            Some(expected) if *expected == '?' || *expected == character => {
                position += 1;
                name = &name[character.len_utf8()..];
            }
            _ => match star {
                Some(star_position) => {
                    let Some(retry_character) = retry_name.chars().next() else {
                        return false;
                    };
                    retry_name = &retry_name[retry_character.len_utf8()..];
                    name = retry_name;
                    position = star_position + 1;
                }
                None => return false,
            },
        }
    }
    pattern[position..]
        .iter()
        .all(|character| *character == '*')
}

#[cfg(test)]
mod tests {
    use super::NameQuery;

    #[test]
    fn masks_match_whole_names_case_insensitively() {
        for name in ["file.exe", "FILE.EXE", ".exe"] {
            assert!(NameQuery::new("*.exe").matches(name));
        }
        for name in ["file.exe.bak", "file.com", "exe"] {
            assert!(!NameQuery::new("*.exe").matches(name));
        }
        assert!(NameQuery::new("file*").matches("file.txt"));
        assert!(!NameQuery::new("file*").matches("other-file.txt"));
        assert!(NameQuery::new("*part*").matches("a-part-b.txt"));
        assert!(NameQuery::new("file?.exe").matches("fileé.exe"));
        assert!(!NameQuery::new("file?.exe").matches("file.exe"));
        assert!(!NameQuery::new("file?.exe").matches("file12.exe"));
    }

    #[test]
    fn alternatives_and_text_terms() {
        let query = NameQuery::new(" ; *.exe ; *.com;; ");
        assert!(query.matches("file.EXE"));
        assert!(query.matches("file.com"));
        assert!(!query.matches("file.txt"));
        assert!(NameQuery::new("report final").matches("FINAL-report.txt"));
        assert!(NameQuery::new("file *.exe;*.com").matches("file-final.exe"));
        assert!(!NameQuery::new("file *.exe;*.com").matches("other.exe"));
        assert!(NameQuery::new("ÁRQUIVO").matches("arquivo-árquivo.txt"));
        assert!(NameQuery::new(" ; ; ").is_empty());
        assert!(!NameQuery::new(" ; ; ").matches("file.txt"));
    }

    #[test]
    fn stars_backtrack_and_allow_empty_sequences() {
        for (mask, name, expected) in [
            ("*", "", true),
            ("**", "file.txt", true),
            ("*ab*bc", "ababc", true),
            ("*ab?c", "ababéc", true),
            ("*ab?c", "ababc", false),
            ("a*b*c", "abc", true),
            ("a*b*c", "ab", false),
            ("?", "é", true),
        ] {
            assert_eq!(
                NameQuery::new(mask).matches(name),
                expected,
                "{mask}: {name}"
            );
        }
    }

    #[test]
    fn literal_runs_preserve_alternative_boundaries() {
        assert_eq!(
            NameQuery::new("file *.exe;*.com;*").literal_alternatives(),
            vec![vec!["file", ".exe"], vec![".com"], vec![]]
        );
    }

    #[test]
    fn wildcard_matching_agrees_with_reference_for_short_inputs() {
        fn reference(pattern: &[char], name: &[char]) -> bool {
            match pattern.split_first() {
                None => name.is_empty(),
                Some(('*', rest)) => {
                    reference(rest, name) || (!name.is_empty() && reference(pattern, &name[1..]))
                }
                Some((character, rest)) => {
                    !name.is_empty()
                        && (*character == '?' || *character == name[0])
                        && reference(rest, &name[1..])
                }
            }
        }
        fn strings(alphabet: &[char], max_length: usize) -> Vec<String> {
            let mut result = vec![String::new()];
            let mut level = vec![String::new()];
            for _ in 0..max_length {
                level = level
                    .iter()
                    .flat_map(|prefix| {
                        alphabet
                            .iter()
                            .map(move |character| format!("{prefix}{character}"))
                    })
                    .collect();
                result.extend(level.iter().cloned());
            }
            result
        }
        let names = strings(&['a', 'b', 'é'], 3);
        for pattern in strings(&['a', 'b', 'é', '*', '?'], 4) {
            let mask: Vec<char> = pattern.chars().collect();
            for name in &names {
                assert_eq!(
                    super::wildcard_matches(&mask, name),
                    reference(&mask, &name.chars().collect::<Vec<_>>()),
                    "pattern={pattern:?}, name={name:?}"
                );
            }
        }
    }
}
