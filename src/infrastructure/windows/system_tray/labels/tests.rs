use super::*;

fn width(value: &str) -> i32 {
    value.chars().count() as i32
}

#[test]
fn short_labels_and_paths_are_preserved() {
    for value in [
        "C:\\Documents",
        "D:\\",
        "Tag: Example",
        "This PC",
        "\\\\server\\share\\Folder",
    ] {
        assert_eq!(compact(value, 80, &width), value);
    }
}

#[test]
fn long_paths_keep_the_root_and_last_whole_components() {
    assert_eq!(
        compact("C:\\Projects\\Archive\\Documents", 20, &width),
        "C:\\…\\Documents"
    );
    assert_eq!(
        compact("C:/Projects/Archive/Documents", 20, &width),
        "C:/…/Documents"
    );
    let unc = compact(
        "\\\\server\\share\\Projects\\Archive\\Documents",
        30,
        &width,
    );
    assert!(unc.starts_with("\\\\server\\share\\…\\"));
    assert!(unc.ends_with("Documents"));
    assert!(width(&unc) <= 30);
}

#[test]
fn very_long_folder_names_shorten_the_middle_of_the_leaf() {
    let value = format!("D:\\Documents\\{}End", "Example".repeat(30));
    let label = compact(&value, 24, &width);
    assert!(label.starts_with("D:\\…\\Example"));
    assert!(label.ends_with("End"));
    assert!(width(&label) <= 24);
}

#[test]
fn generic_labels_and_unicode_are_bounded_without_splitting_utf8() {
    for value in [
        "Tag: Example very long tag name",
        "標籤：資料資料資料資料資料",
        "Folder 📁📁📁📁📁📁📁📁📁 End",
    ] {
        let label = compact(value, 12, &width);
        assert!(width(&label) <= 12);
        assert!(label.contains('…'));
        assert!(!label.contains('\u{fffd}'));
    }
    assert_eq!(compact("Example", 0, &width), "");
    assert_eq!(compact("Example", 1, &width), "…");
    assert_eq!(compact("A\tB\nC\0", 30, &width), "A B C ");
}

#[test]
fn measurement_not_character_count_controls_the_budget() {
    let pixels = |s: &str| {
        s.chars()
            .map(|c| if c == 'W' { 15 } else { 3 })
            .sum::<i32>()
    };
    assert_eq!(compact("iiiiiiii", 30, &pixels), "iiiiiiii");
    let wide = compact("WWWWWWWW", 30, &pixels);
    assert_ne!(wide, "WWWWWWWW");
    assert!(pixels(&wide) <= 30);
}

#[test]
fn identical_shortened_paths_gain_parent_context() {
    let leaf = "LongExampleFolder".repeat(10);
    let first = format!("C:\\Alpha\\{leaf}");
    let second = format!("C:\\Beta\\{leaf}");
    assert_eq!(compact(&first, 32, &width), compact(&second, 32, &width));
    let labels = siblings(&[&first, &second], 32, &width);
    assert_ne!(labels[0], labels[1]);
    assert!(labels[0].contains("Alpha"));
    assert!(labels[1].contains("Beta"));
    assert!(labels.iter().all(|s| width(s) <= 32));
}

#[test]
fn residual_collisions_use_bounded_ordinals_and_keep_duplicate_paths_equal() {
    let values = [
        "Tag: Beginning one omitted middle with a common ending",
        "Tag: Beginning two omitted middle with a common ending",
        "Tag: Beginning one omitted middle with a common ending",
        "Another tag [1]",
    ];
    let labels = siblings(&values, 20, &width);
    assert_ne!(labels[0], labels[1]);
    assert_ne!(labels[0], labels[3]);
    assert_eq!(labels[0], labels[2]);
    assert!(labels.iter().all(|s| width(s) <= 20));
    assert_eq!(
        values[0],
        "Tag: Beginning one omitted middle with a common ending"
    );
}

#[test]
fn duplicate_tabs_and_distinct_short_labels_do_not_gain_ordinals() {
    let values = ["C:\\Documents", "C:\\Documents", "D:\\Videos"];
    assert_eq!(siblings(&values, 30, &width), values);
}

#[test]
fn every_budget_bounds_drive_unc_and_generic_labels() {
    let values = [
        "C:\\Documents\\Example\\Archive\\VeryLongFolderName",
        "\\\\server\\share\\Example\\Archive\\VeryLongFolderName",
        "標籤：資料資料資料資料資料資料資料資料資料",
        "Tag: Example very long label with a common ending",
    ];
    for budget in 0..100 {
        for value in values {
            assert!(width(&compact(value, budget, &width)) <= budget);
        }
        assert!(siblings(&values, budget, &width)
            .iter()
            .all(|s| width(s) <= budget));
    }
}

#[test]
fn window_labels_omit_prefixes_and_only_distinguish_ambiguous_groups() {
    let distinct = [r"C:\Documents", r"D:\Videos"];
    assert_eq!(windows(&distinct, 40, &width), distinct);
    let repeated = [r"C:\Documents", r"C:\Documents", r"D:\Videos"];
    let labels = windows(&repeated, 40, &width);
    assert_eq!(
        labels,
        [r"C:\Documents [1]", r"C:\Documents [2]", r"D:\Videos"]
    );
    let literal_ordinal = ["Example", "Example", "Example [1]"];
    let labels = windows(&literal_ordinal, 40, &width);
    let unique = labels.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(unique.len(), labels.len());
    for budget in 0..40 {
        assert!(windows(&repeated, budget, &width)
            .iter()
            .all(|s| width(s) <= budget));
    }
}
