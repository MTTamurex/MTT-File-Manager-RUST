use super::*;

#[cfg(test)]
mod tests;

pub(super) fn values(
    model: &DiskAnalysisModel,
    idx: u32,
    filtered: Option<(SizeMetric, u64)>,
) -> [String; 4] {
    let node = &model.nodes[idx as usize];
    let value = |metric, natural| {
        filtered
            .filter(|(active, _)| *active == metric)
            .map_or(natural, |(_, value)| value)
    };
    [
        format_size(value(SizeMetric::Logical, node.subtree_size)),
        format_size(value(SizeMetric::Allocated, node.subtree_allocated_size)),
        signed_diff_text(node.subtree_size, node.subtree_allocated_size),
        if node.is_dir {
            value(SizeMetric::FileCount, node.subtree_files).to_string()
        } else {
            String::new()
        },
    ]
}

pub(super) fn filtered(state: &DiskAnalysisState, idx: u32) -> Option<(SizeMetric, u64)> {
    state
        .active_weights
        .as_ref()?
        .weights
        .get(idx as usize)
        .map(|&v| (state.metric, v))
}

pub(super) fn widths(
    ui: &egui::Ui,
    available: f32,
    model: &DiskAnalysisModel,
    state: &DiskAnalysisState,
) -> [f32; 7] {
    let measure = |text: &str| {
        ui.painter()
            .layout_no_wrap(
                text.to_owned(),
                egui::FontId::proportional(TABLE_FONT_SIZE),
                analyzer_text_color(ui),
            )
            .size()
            .x
            + 14.0
    };
    let mut widths = [
        0.0,
        0.0,
        measure(&format!("{} ↓", t!("disk_analysis.col_type"))),
        measure(&format!("{} ↓", t!("disk_analysis.logical_size_short"))),
        measure(&format!("{} ↓", t!("disk_analysis.allocated_size"))),
        measure(&format!("{} ↓", t!("disk_analysis.col_difference"))),
        measure(&format!("{} ↓", t!("disk_analysis.col_files"))),
    ];
    for text in [
        t!("disk_analysis.type_file"),
        t!("disk_analysis.type_folder"),
    ] {
        widths[2] = widths[2].max(measure(&text));
    }
    // Only the bounded result projection is measured, never the whole tree or
    // full paths. The same formatting is used when the visible rows are painted.
    for &idx in state.largest_rows.iter() {
        for (slot, text) in values(model, idx, filtered(state, idx)).iter().enumerate() {
            widths[slot + 3] = widths[slot + 3].max(measure(text));
        }
    }
    let flex = (available - widths[2..].iter().sum::<f32>()).max(220.0);
    widths[0] = (flex * 0.42).clamp(120.0, 320.0);
    widths[1] = (flex - widths[0]).max(80.0);
    widths
}
