/// Move a tab to a gap in the original order while keeping the same tab active.
pub(super) fn move_to_gap<T>(
    tabs: &mut Vec<T>,
    active: &mut usize,
    from: usize,
    gap: usize,
) -> bool {
    if from >= tabs.len() || gap > tabs.len() {
        return false;
    }
    let insert = if gap > from { gap - 1 } else { gap };
    if insert == from {
        return false;
    }
    let tab = tabs.remove(from);
    tabs.insert(insert, tab);
    if *active == from {
        *active = insert;
    } else if from < *active && insert >= *active {
        *active -= 1;
    } else if from > *active && insert <= *active {
        *active += 1;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reorder_preserves_active_identity_for_every_source_and_gap() {
        for selected in 0..4 {
            for from in 0..4 {
                for gap in 0..=4 {
                    let mut tabs = vec![0, 1, 2, 3];
                    let mut active = selected;
                    let changed = move_to_gap(&mut tabs, &mut active, from, gap);
                    assert_eq!(tabs[active], selected);
                    assert_eq!(changed, gap != from && gap != from + 1);
                    if changed {
                        let destination = if gap > from { gap - 1 } else { gap };
                        assert_eq!(tabs[destination], from);
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_moves_leave_tabs_unchanged() {
        let mut tabs = vec![0, 1, 2];
        let mut active = 1;
        assert!(!move_to_gap(&mut tabs, &mut active, 3, 0));
        assert!(!move_to_gap(&mut tabs, &mut active, 0, 4));
        assert_eq!(tabs, [0, 1, 2]);
        assert_eq!(active, 1);
        assert!(!move_to_gap::<usize>(&mut Vec::new(), &mut 0, 0, 0));
    }
}
