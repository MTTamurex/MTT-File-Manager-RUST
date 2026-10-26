//! Defer tab mutations until both physical panels have finished rendering.
use crate::app::dual_panel::ActivePanel;
use crate::domain::file_entry::FileEntry;
use eframe::egui;

#[derive(Clone)]
pub(crate) struct NewTabRequest {
    pub panel: ActivePanel,
    pub paths: Vec<String>,
    pub ctrl: bool,
}

pub(crate) fn set_render_panel(ctx: &egui::Context, panel: ActivePanel, enabled: bool) {
    ctx.data_mut(|data| {
        data.insert_temp(
            egui::Id::new("new_tab_render_panel"),
            enabled.then_some(panel),
        )
    });
}

pub(crate) fn is_rendering_panel(ctx: &egui::Context) -> bool {
    ctx.data(|data| {
        data.get_temp::<Option<ActivePanel>>(egui::Id::new("new_tab_render_panel"))
            .flatten()
            .is_some()
    })
}

pub(crate) fn item_middle_click(response: &egui::Response, item: &FileEntry, recycle_bin: bool) {
    if recycle_bin || item.is_recycle_item() || !item.is_dir || !response.middle_clicked() {
        return;
    }
    queue(
        &response.ctx,
        &item.path.to_string_lossy(),
        response.ctx.input(|input| input.modifiers.ctrl),
    );
}

pub(crate) fn queue(ctx: &egui::Context, path: &str, ctrl: bool) {
    queue_paths(ctx, vec![path.to_owned()], ctrl);
}

pub(crate) fn queue_paths(ctx: &egui::Context, paths: Vec<String>, ctrl: bool) {
    if paths.is_empty() {
        return;
    }
    ctx.data_mut(|data| {
        if let Some(panel) = data
            .get_temp::<Option<ActivePanel>>(egui::Id::new("new_tab_render_panel"))
            .flatten()
        {
            data.insert_temp(
                egui::Id::new("new_tab_request"),
                NewTabRequest { panel, paths, ctrl },
            );
        }
    });
}

pub(crate) fn take(ctx: &egui::Context) -> Option<NewTabRequest> {
    ctx.data_mut(|data| {
        let id = egui::Id::new("new_tab_request");
        let request = data.get_temp::<NewTabRequest>(id);
        data.remove::<NewTabRequest>(id);
        request
    })
}

#[cfg(test)]
mod tests;
