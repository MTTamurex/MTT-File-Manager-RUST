//! Prefix navigation without changing or filtering the installed-font list.

const RESET_AFTER_SECONDS: f64 = 1.0;

pub(super) enum Edit {
    Text(String),
    Backspace,
}

#[derive(Clone, Default)]
pub(super) struct TypeAhead {
    text: String,
    last_input: Option<f64>,
}

impl TypeAhead {
    pub fn clear(&mut self) {
        self.text.clear();
        self.last_input = None;
    }

    pub fn input(&mut self, text: &str, backspace: bool, now: f64) -> Option<String> {
        if text.is_empty() && !backspace {
            return None;
        }
        if self
            .last_input
            .is_none_or(|last| now - last > RESET_AFTER_SECONDS)
        {
            self.text.clear();
        }
        if backspace {
            self.text.pop();
        }
        self.text.extend(text.chars().filter(|ch| !ch.is_control()));
        self.last_input = Some(now);
        (!self.text.is_empty()).then(|| self.text.to_lowercase())
    }
}

// Catalog families are sorted by their lowercase names. Keep lookup logarithmic
// even for machines with many installed fonts; the default option stays first.
pub(super) fn find(families: &[String], default_label: &str, prefix: &str) -> Option<usize> {
    if default_label.to_lowercase().starts_with(prefix) {
        return Some(0);
    }
    let index = families.partition_point(|name| name.to_lowercase().as_str() < prefix);
    families
        .get(index)
        .filter(|name| name.to_lowercase().starts_with(prefix))
        .map(|_| index + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_accumulates_and_a_pause_starts_a_new_prefix() {
        let mut search = TypeAhead::default();
        assert_eq!(search.input("Te", false, 0.0).as_deref(), Some("te"));
        assert_eq!(search.input("st", false, 0.2).as_deref(), Some("test"));
        assert_eq!(search.input("Other", false, 2.0).as_deref(), Some("other"));
        search.clear();
        assert_eq!(search.input("New", false, 2.1).as_deref(), Some("new"));
    }

    #[test]
    fn spaces_unicode_and_backspace_are_preserved() {
        let mut search = TypeAhead::default();
        assert_eq!(
            search.input("Árvore Font", false, 0.0).as_deref(),
            Some("árvore font")
        );
        assert_eq!(search.input("", true, 0.1).as_deref(), Some("árvore fon"));
    }

    #[test]
    fn prefix_matches_the_first_family_without_filtering_the_list() {
        let families = vec![
            "Other Font".into(),
            "Test Font A".into(),
            "Test Font B".into(),
            "Árvore Font".into(),
        ];
        assert_eq!(find(&families, "Default", "test"), Some(2));
        assert_eq!(find(&families, "Default", "test font b"), Some(3));
        assert_eq!(find(&families, "Default", "ár"), Some(4));
        assert_eq!(find(&families, "Default", "def"), Some(0));
        assert_eq!(find(&families, "Default", "missing"), None);
        assert_eq!(families.len(), 4);
    }
}
