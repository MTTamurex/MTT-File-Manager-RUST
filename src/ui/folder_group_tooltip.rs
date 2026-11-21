//! Hover text shared by both Quick Access surfaces for saved tab groups.

use crate::domain::file_tag::FileTag;
use crate::domain::folder_group::{FolderGroup, SavedGroupPanelTabs, SavedGroupTab};
use crate::domain::special_paths::{tag_id_from_view_path, COMPUTER_VIEW_ID, RECYCLE_BIN_VIEW_ID};
use eframe::egui;
use rust_i18n::t;
use rustc_hash::FxHashMap;
use std::path::Path;

pub(crate) fn render(
    ui: &mut egui::Ui,
    group: &FolderGroup,
    tag_definitions: &FxHashMap<i64, FileTag>,
) {
    ui.strong(&group.name);
    let lines = tab_lines(group, tag_definitions);
    if lines.is_empty() {
        return;
    }
    ui.add_space(2.0);
    for line in lines {
        ui.label(line);
    }
}

fn tab_lines(group: &FolderGroup, tag_definitions: &FxHashMap<i64, FileTag>) -> Vec<String> {
    if let Some(saved) = group
        .saved_tabs
        .as_ref()
        .filter(|saved| !saved.tabs.is_empty())
    {
        saved
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| format!("• {}", tab_label(index, tab, tag_definitions)))
            .collect()
    } else {
        group
            .paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                format!(
                    "• {}: {}",
                    t!("tray.tab_number", number = index + 1),
                    path_label(path, tag_definitions)
                )
            })
            .collect()
    }
}

fn tab_label(
    index: usize,
    tab: &SavedGroupTab,
    tag_definitions: &FxHashMap<i64, FileTag>,
) -> String {
    let tab_number = t!("tray.tab_number", number = index + 1);
    match &tab.dual_panel {
        Some(dual) => format!(
            "{} — {}: {} · {}: {}",
            tab_number,
            t!("tray.left_panel"),
            panel_label(&dual.left, tag_definitions),
            t!("tray.right_panel"),
            panel_label(&dual.right, tag_definitions)
        ),
        None => format!("{}: {}", tab_number, path_label(&tab.path, tag_definitions)),
    }
}

fn panel_label(panel: &SavedGroupPanelTabs, tag_definitions: &FxHashMap<i64, FileTag>) -> String {
    panel
        .tabs
        .iter()
        .enumerate()
        .map(|(index, tab)| {
            let label = path_label(&tab.path, tag_definitions);
            if index == panel.active {
                format!("[{label}]")
            } else {
                label
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn path_label(path: &str, tag_definitions: &FxHashMap<i64, FileTag>) -> String {
    match path {
        COMPUTER_VIEW_ID => t!("nav.computer").into_owned(),
        RECYCLE_BIN_VIEW_ID => t!("nav.recycle_bin").into_owned(),
        _ => {
            if let Some(tag_id) = tag_id_from_view_path(path) {
                crate::app::operations::tag_ops::tag_view_title(tag_id, tag_definitions)
            } else {
                Path::new(path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_owned())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::folder_group::{
        SavedGroupDualPanel, SavedGroupPanel, SavedGroupPanelTab, SavedGroupTabs,
    };

    fn panel(active: usize, paths: &[&str]) -> SavedGroupPanelTabs {
        SavedGroupPanelTabs {
            active,
            tabs: paths
                .iter()
                .map(|path| SavedGroupPanelTab {
                    path: (*path).to_owned(),
                    pinned: false,
                })
                .collect(),
        }
    }

    #[test]
    fn mono_groups_list_every_saved_tab_title() {
        let group = FolderGroup {
            id: 1,
            name: "Work".to_owned(),
            position: 0,
            paths: vec![
                r"C:\Users\user\Documents\Project".to_owned(),
                r"D:\Videos\Folder".to_owned(),
            ],
            saved_tabs: Some(SavedGroupTabs {
                active_tab: 0,
                tabs: vec![
                    SavedGroupTab {
                        path: r"C:\Users\user\Documents\Project".to_owned(),
                        dual_panel: None,
                    },
                    SavedGroupTab {
                        path: r"D:\Videos\Folder".to_owned(),
                        dual_panel: None,
                    },
                ],
            }),
        };

        assert_eq!(
            tab_lines(&group, &FxHashMap::default()),
            [
                format!("• {}: Project", t!("tray.tab_number", number = 1)),
                format!("• {}: Folder", t!("tray.tab_number", number = 2))
            ]
        );
    }

    #[test]
    fn dual_panel_groups_show_the_active_title_from_each_side() {
        let group = FolderGroup {
            id: 1,
            name: "Workspace".to_owned(),
            position: 0,
            paths: vec![r"C:\Users\user\Documents\Project\FolderB".to_owned()],
            saved_tabs: Some(SavedGroupTabs {
                active_tab: 0,
                tabs: vec![SavedGroupTab {
                    path: r"C:\Users\user\Documents\Project\FolderB".to_owned(),
                    dual_panel: Some(SavedGroupDualPanel {
                        active: SavedGroupPanel::Left,
                        split_ratio: 0.5,
                        left: panel(
                            1,
                            &[
                                r"C:\Users\user\Documents\Project\FolderA",
                                r"C:\Users\user\Documents\Project\FolderB",
                            ],
                        ),
                        right: panel(0, &[r"D:\Videos\FolderA"]),
                    }),
                }],
            }),
        };

        assert_eq!(
            tab_lines(&group, &FxHashMap::default()),
            [format!(
                "• {} — {}: FolderA · [FolderB] · {}: [FolderA]",
                t!("tray.tab_number", number = 1),
                t!("tray.left_panel"),
                t!("tray.right_panel")
            )]
        );
    }

    #[test]
    fn legacy_groups_fall_back_to_their_stored_paths() {
        let group = FolderGroup {
            id: 1,
            name: "Legacy".to_owned(),
            position: 0,
            paths: vec![COMPUTER_VIEW_ID.to_owned()],
            saved_tabs: None,
        };

        assert_eq!(
            tab_lines(&group, &FxHashMap::default()),
            [format!(
                "• {}: {}",
                t!("tray.tab_number", number = 1),
                t!("nav.computer")
            )]
        );
    }
}
