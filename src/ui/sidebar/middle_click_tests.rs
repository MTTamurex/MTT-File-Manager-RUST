use super::*;
use crate::domain::file_tag::TagColor;
use crate::infrastructure::directory_cache::DirectoryCache;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum Section {
    Cloud,
    Local,
    Tag,
    NetworkLocation,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Cloud => "Cloud Root",
            Self::Local => "Local Disk",
            Self::Tag => "Tag Name",
            Self::NetworkLocation => "Network Folder",
        }
    }

    fn destination(self) -> String {
        match self {
            Self::Cloud => r"C:\Cloud".to_string(),
            Self::Local => "C:\\".to_string(),
            Self::Tag => crate::domain::special_paths::tag_view_path(7),
            Self::NetworkLocation => r"\\server\share\folder".into(),
        }
    }
}

fn click_section(
    section: Section,
    button: egui::PointerButton,
    renaming: bool,
    dragging: bool,
) -> Option<SidebarAction> {
    click_section_with_feedback(section, button, renaming, dragging).0
}

fn click_section_with_feedback(
    section: Section,
    button: egui::PointerButton,
    renaming: bool,
    dragging: bool,
) -> (Option<SidebarAction>, bool) {
    let ctx = egui::Context::default();
    let mut fonts = egui::FontDefinitions::default();
    fonts.families.insert(
        egui::FontFamily::Name("icons".into()),
        fonts.families[&egui::FontFamily::Proportional].clone(),
    );
    ctx.set_fonts(fonts);
    let disks = vec![("C:\\".to_string(), Section::Local.label().to_string())];
    let clouds = vec![CloudRoot::windows_cloud_files(
        Section::Cloud.destination(),
        Section::Cloud.label().to_string(),
        None,
    )];
    let network_locations = vec![crate::domain::network_location::NetworkLocation {
        path: Section::NetworkLocation.destination(),
        label: Section::NetworkLocation.label().into(),
    }];
    let tags = FxHashMap::from_iter([(
        7,
        FileTag {
            id: 7,
            name: Section::Tag.label().to_string(),
            color: TagColor::Blue,
            position: 0,
        },
    )]);
    let tree = SidebarTreeState::new(Arc::new(DirectoryCache::new()));
    let drive_info = HashMap::new();
    let mounted_iso_drives = HashMap::new();
    let tag_counts = FxHashMap::default();
    let mut icon_loader = crate::ui::icon_loader::IconLoader::new();
    let mut render = |ui: &mut egui::Ui| {
        ui.set_width(260.0);
        let mut sidebar = SidebarContext {
            disks: &disks,
            drive_info_cache: &drive_info,
            cloud_roots: &clouds,
            network_locations: &network_locations,
            current_path: "D:\\",
            context_selection: None,
            is_computer_view: false,
            is_recycle_bin_view: false,
            computer_icon: None,
            is_renaming: renaming,
            icon_loader: &mut icon_loader,
            pinned_folders: &[],
            is_item_dragging: dragging,
            can_drop_items_to_recycle_bin: false,
            is_folder_dragging: false,
            dragging_path: None,
            show_recycle_bin: false,
            show_quick_access: false,
            show_tags: true,
            collapse_quick_access: false,
            collapse_cloud_drives: false,
            collapse_local_disks: false,
            collapse_network_drives: false,
            sidebar_renaming: None,
            sidebar_rename_focus: false,
            folder_rename: None,
            mounted_iso_drives: &mounted_iso_drives,
            tree_state: &tree,
            tag_definitions: &tags,
            sidebar_tag_ids: &[7],
            tag_counts: &tag_counts,
            active_tag_filter: Some(7),
            collapse_tags: false,
        };
        match section {
            Section::Cloud | Section::Local | Section::NetworkLocation => {
                render_sidebar_drives(ui, &mut sidebar, 260.0)
            }
            Section::Tag => {
                let mut action = None;
                render_tags_section(ui, &mut sidebar, &mut action);
                action
            }
        }
    };
    let output = ctx.run_ui(egui::RawInput::default(), |ui| {
        let _ = render(ui);
    });
    let pos = output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::epaint::Shape::Text(text) if text.galley.text() == section.label() => {
                Some(shape.shape.visual_bounding_rect().center())
            }
            _ => None,
        })
        .expect("section item is visible");
    let mut result = None;
    let mut highlighted = false;
    for pressed in [true, false] {
        let output = ctx.run_ui(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ui| {
                if let Some(action) = render(ui) {
                    result = Some(action);
                }
            },
        );
        if !pressed {
            let color = crate::ui::theme::selection_color(ctx.global_style().visuals.dark_mode);
            highlighted = output.shapes.iter().any(|shape| {
                matches!(&shape.shape,
                egui::epaint::Shape::Rect(rect) if rect.fill == color && rect.rect.contains(pos))
            });
        }
    }
    (result, highlighted)
}

#[test]
fn sidebar_right_click_immediately_highlights_the_drive_or_cloud_root_without_navigation() {
    let (action, highlighted) =
        click_section_with_feedback(Section::Local, egui::PointerButton::Secondary, false, false);
    assert!(
        matches!(action, Some(SidebarAction::OpenDriveContextMenu(path)) if path == Section::Local.destination())
    );
    assert!(highlighted);
    let (action, highlighted) =
        click_section_with_feedback(Section::Cloud, egui::PointerButton::Secondary, false, false);
    assert!(
        matches!(action, Some(SidebarAction::OpenTreeContextMenu(target)) if target.path == std::path::Path::new(&Section::Cloud.destination()))
    );
    assert!(highlighted);
}

#[test]
fn network_locations_support_navigation_and_middle_click_without_remote_metadata() {
    let section = Section::NetworkLocation;
    assert!(
        matches!(click_section(section, egui::PointerButton::Primary, false, false), Some(SidebarAction::NavigateTo(path)) if path == section.destination())
    );
    assert!(
        matches!(click_section(section, egui::PointerButton::Middle, false, false), Some(SidebarAction::OpenInNewTab(path)) if path == section.destination())
    );
    assert!(click_section(section, egui::PointerButton::Middle, true, false).is_none());
    assert!(click_section(section, egui::PointerButton::Middle, false, true).is_none());
}

#[test]
fn middle_click_opens_cloud_local_and_tag_destinations_in_new_tabs() {
    for section in [Section::Cloud, Section::Local, Section::Tag] {
        let action = click_section(section, egui::PointerButton::Middle, false, false);
        assert!(
            matches!(action, Some(SidebarAction::OpenInNewTab(path)) if path == section.destination())
        );
    }
}

#[test]
fn primary_click_keeps_navigation_and_tag_toggle_behavior() {
    for section in [Section::Cloud, Section::Local] {
        let action = click_section(section, egui::PointerButton::Primary, false, false);
        assert!(
            matches!(action, Some(SidebarAction::NavigateTo(path)) if path == section.destination())
        );
    }
    assert!(matches!(
        click_section(Section::Tag, egui::PointerButton::Primary, false, false),
        Some(SidebarAction::FilterByTag(None))
    ));
}

#[test]
fn middle_click_does_not_open_tabs_during_rename_or_item_drag() {
    for section in [Section::Cloud, Section::Local, Section::Tag] {
        for (renaming, dragging) in [(true, false), (false, true)] {
            assert!(
                click_section(section, egui::PointerButton::Middle, renaming, dragging).is_none()
            );
        }
    }
}
