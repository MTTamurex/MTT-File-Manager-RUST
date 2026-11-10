use super::catalog::{Command, COMMANDS};
use super::input_hints;
use crate::app::shortcuts::ShortcutBindings;
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

pub(crate) struct SearchEntry {
    pub command: Command,
    pub label: String,
    pub inputs: String,
}

pub(crate) fn normalize(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn score(label: &str, category: &str, query: &str) -> Option<u8> {
    if query.is_empty() {
        return Some(0);
    }
    let label = normalize(label);
    let haystack = format!("{label} {}", normalize(category));
    if !query.split_whitespace().all(|word| haystack.contains(word)) {
        return None;
    }
    Some(if label == query {
        0
    } else if label.starts_with(query) {
        1
    } else if label.contains(query) {
        2
    } else {
        3
    })
}

pub(crate) fn entries(query: &str, shortcuts: &ShortcutBindings) -> Vec<SearchEntry> {
    let query = normalize(query.trim());
    let mut results: Vec<_> = COMMANDS
        .iter()
        .filter_map(|&command| {
            let label = rust_i18n::t!(command.label_key()).to_string();
            let category = rust_i18n::t!(command.category_key()).to_string();
            let inputs = input_hints::summary(command, shortcuts);
            let searchable = format!("{category} {inputs}");
            score(&label, &searchable, &query).map(|score| {
                (
                    score,
                    SearchEntry {
                        command,
                        label,
                        inputs,
                    },
                )
            })
        })
        .collect();
    results.sort_by_key(|(score, _)| *score);
    results.into_iter().map(|(_, entry)| entry).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_case_accents_and_word_order() {
        assert!(score(
            "Inverter seleção",
            "Seleção",
            &normalize("SELECAO inverter")
        )
        .is_some());
        assert_eq!(normalize("SELEC\u{0327}A\u{0303}O"), "selecao");
        assert_eq!(normalize("ΑΝΑΖΉΤΗΣΗ"), "αναζητηση");
        assert_eq!(normalize("搜索"), "搜索");
    }

    #[test]
    fn ranking_and_missing_words() {
        assert_eq!(score("Settings", "Tools", "settings"), Some(0));
        assert_eq!(score("Settings window", "Tools", "settings"), Some(1));
        assert_eq!(score("Open settings", "Tools", "settings"), Some(2));
        assert_eq!(score("Open settings", "Tools", "tools open"), Some(3));
        assert_eq!(score("Open settings", "Tools", "missing"), None);
        assert_eq!(score("Open settings", "Tools", ""), Some(0));
    }

    #[test]
    fn mouse_hints_and_remapped_view_shortcuts_are_searchable() {
        let mut shortcuts = ShortcutBindings::default();
        let mouse = rust_i18n::t!("command_palette.mouse_back");
        assert!(entries(&mouse, &shortcuts)
            .iter()
            .any(|entry| entry.command == Command::Back));
        shortcuts.set(
            crate::app::shortcuts::ShortcutAction::NextViewMode,
            crate::app::shortcuts::ShortcutBinding::ctrl_shift(eframe::egui::Key::G),
        );
        let found = entries("Ctrl+Shift+G", &shortcuts);
        assert!(found.iter().any(|entry| entry.command == Command::NextView));
        assert!(found.iter().any(|entry| entry.command == Command::Grid));
    }
}
