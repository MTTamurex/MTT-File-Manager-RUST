use serde::{Deserialize, Serialize};

/// A named group of workspace tab paths shown in Quick Access.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderGroup {
    pub id: i64,
    pub name: String,
    pub position: i64,
    pub paths: Vec<String>,
    /// Full workspace state for groups saved after dual-panel support.
    /// `None` for legacy groups that only stored one path per outer tab.
    pub saved_tabs: Option<SavedGroupTabs>,
}

/// Serialized workspace state for one saved tab group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGroupTabs {
    #[serde(default)]
    pub active_tab: usize,
    pub tabs: Vec<SavedGroupTab>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGroupTab {
    pub path: String,
    #[serde(default)]
    pub dual_panel: Option<SavedGroupDualPanel>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedGroupDualPanel {
    pub active: SavedGroupPanel,
    pub split_ratio: f32,
    pub left: SavedGroupPanelTabs,
    pub right: SavedGroupPanelTabs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SavedGroupPanel {
    Left,
    Right,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedGroupPanelTabs {
    #[serde(default)]
    pub active: usize,
    pub tabs: Vec<SavedGroupPanelTab>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedGroupPanelTab {
    pub path: String,
    #[serde(default)]
    pub pinned: bool,
}

impl SavedGroupTabs {
    /// Rejects indices and ratios that would be invalid at restore time.
    pub fn sanitize(&mut self) {
        if self.tabs.is_empty() {
            self.active_tab = 0;
            return;
        }
        self.active_tab = self.active_tab.min(self.tabs.len() - 1);

        for tab in &mut self.tabs {
            let invalid_dual_panel = match tab.dual_panel.as_mut() {
                Some(dual) => {
                    if !dual.split_ratio.is_finite() {
                        dual.split_ratio = 0.5;
                    }
                    dual.split_ratio = dual.split_ratio.clamp(0.0, 1.0);
                    dual.left.sanitize();
                    dual.right.sanitize();
                    dual.left.tabs.is_empty() || dual.right.tabs.is_empty()
                }
                None => false,
            };
            if invalid_dual_panel {
                tab.dual_panel = None;
            }
        }
    }
}

impl SavedGroupPanelTabs {
    fn sanitize(&mut self) {
        self.active = if self.tabs.is_empty() {
            0
        } else {
            self.active.min(self.tabs.len() - 1)
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panel(path: &str) -> SavedGroupPanelTabs {
        SavedGroupPanelTabs {
            active: 5,
            tabs: vec![SavedGroupPanelTab {
                path: path.to_owned(),
                pinned: false,
            }],
        }
    }

    #[test]
    fn sanitize_clamps_indices_and_split_ratio() {
        let mut saved = SavedGroupTabs {
            active_tab: 9,
            tabs: vec![SavedGroupTab {
                path: "C:\\Users\\user\\Documents\\Project".to_owned(),
                dual_panel: Some(SavedGroupDualPanel {
                    active: SavedGroupPanel::Right,
                    split_ratio: 4.0,
                    left: panel("C:\\Users\\user\\Documents\\Project\\Folder"),
                    right: panel("D:\\Videos\\Folder"),
                }),
            }],
        };

        saved.sanitize();

        assert_eq!(saved.active_tab, 0);
        let dual = saved.tabs[0].dual_panel.as_ref().unwrap();
        assert_eq!(dual.split_ratio, 1.0);
        assert_eq!(dual.left.active, 0);
        assert_eq!(dual.right.active, 0);
    }

    #[test]
    fn sanitize_drops_dual_panels_with_an_empty_side() {
        let mut saved = SavedGroupTabs {
            active_tab: 0,
            tabs: vec![SavedGroupTab {
                path: "C:\\Users\\user\\Documents\\Project".to_owned(),
                dual_panel: Some(SavedGroupDualPanel {
                    active: SavedGroupPanel::Left,
                    split_ratio: 0.5,
                    left: panel("C:\\Users\\user\\Documents\\Project\\Folder"),
                    right: SavedGroupPanelTabs {
                        active: 0,
                        tabs: Vec::new(),
                    },
                }),
            }],
        };

        saved.sanitize();

        assert!(saved.tabs[0].dual_panel.is_none());
    }

    #[test]
    fn sanitize_preserves_wide_and_narrow_panel_ratios() {
        for ratio in [0.1, 0.9] {
            let mut saved = SavedGroupTabs {
                active_tab: 0,
                tabs: vec![SavedGroupTab {
                    path: "C:\\".to_owned(),
                    dual_panel: Some(SavedGroupDualPanel {
                        active: SavedGroupPanel::Left,
                        split_ratio: ratio,
                        left: panel("C:\\"),
                        right: panel("D:\\"),
                    }),
                }],
            };
            saved.sanitize();
            assert_eq!(
                saved.tabs[0].dual_panel.as_ref().unwrap().split_ratio,
                ratio
            );
        }
    }
}
