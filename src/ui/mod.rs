// UI components and rendering logic

pub(crate) mod address_bar;
pub mod app; // NEW - Modular sub-apps
pub mod app_impl; // NEW - eframe::App implementation
pub mod cache;
pub(crate) mod clipboard_feedback;
pub mod components;
pub mod context_menu;
pub(crate) mod disk_analysis;
pub(crate) mod drive_health_badge;
pub(crate) mod folder_appearance;
pub mod fonts;
pub mod global_search_overlay;

pub mod icon_loader;
pub(crate) mod interface_scale;
pub mod navigation;
pub(crate) mod new_tab_request;
pub mod preview_panel;
pub mod sidebar;
pub mod sidebar_cloud_roots;
pub(crate) mod sidebar_network_locations;
pub mod sidebar_selection;
pub mod sidebar_tree;
pub mod status_bar;
pub mod svg_icons;
pub mod tab_bar;
pub(crate) mod tag_icon;
pub mod theme;
pub mod toolbar;
pub(crate) mod video_overlay;
pub mod views;
pub mod widgets;
