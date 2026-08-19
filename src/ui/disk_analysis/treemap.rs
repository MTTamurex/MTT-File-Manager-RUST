//! Squarified treemap layout and painting for the disk analyzer view.
//!
//! The layout is recomputed per frame over the current drill-down root only;
//! rectangles below pixel thresholds are pruned so the placed list stays
//! small (hundreds of entries) regardless of volume size.

use crate::app::disk_analysis_model::DiskAnalysisModel;
use eframe::egui;

pub const HEADER_HEIGHT: f32 = 20.0;
const MIN_FRAME_W: f32 = 70.0;
const MIN_FRAME_H: f32 = 48.0;
const MIN_LEAF_SIZE: f32 = 3.0;
const MAX_DEPTH: u32 = 5;
const GAP: f32 = 2.0;

#[derive(Clone, Copy)]
pub struct PlacedRect {
    pub idx: u32,
    pub rect: egui::Rect,
    pub is_dir: bool,
    /// Whether the frame is large enough for a name/size header band.
    pub header: bool,
}

/// Layout the children of `root_idx` into `area`. Parents are emitted before
/// their children so hit-testing can pick the deepest entry containing a point.
pub fn layout(model: &DiskAnalysisModel, root_idx: u32, area: egui::Rect) -> Vec<PlacedRect> {
    let mut out = Vec::new();
    layout_children(model, root_idx, area, 0, &mut out);
    out
}

fn weight(model: &DiskAnalysisModel, idx: u32) -> f32 {
    let node = &model.nodes[idx as usize];
    let value = if node.is_dir {
        node.subtree_size
    } else {
        node.size
    };
    value.max(1) as f32
}

fn layout_children(
    model: &DiskAnalysisModel,
    parent: u32,
    rect: egui::Rect,
    depth: u32,
    out: &mut Vec<PlacedRect>,
) {
    if rect.width() < MIN_LEAF_SIZE * 2.0 || rect.height() < MIN_LEAF_SIZE * 2.0 {
        return;
    }
    let children: Vec<(u32, f32)> = model
        .children(parent)
        .iter()
        .map(|&child| (child, weight(model, child)))
        .filter(|(_, w)| *w > 0.0)
        .collect();
    if children.is_empty() {
        return;
    }

    for (idx, r) in squarify(&children, rect) {
        let node = &model.nodes[idx as usize];
        if node.is_dir {
            let header = r.width() >= MIN_FRAME_W && r.height() >= MIN_FRAME_H;
            out.push(PlacedRect {
                idx,
                rect: r,
                is_dir: true,
                header,
            });
            if header && depth < MAX_DEPTH && !node.is_reparse {
                let content = egui::Rect::from_min_max(
                    egui::pos2(r.min.x + GAP, r.min.y + HEADER_HEIGHT),
                    egui::pos2(r.max.x - GAP, r.max.y - GAP),
                );
                layout_children(model, idx, content, depth + 1, out);
            }
        } else if r.width() >= MIN_LEAF_SIZE && r.height() >= MIN_LEAF_SIZE {
            out.push(PlacedRect {
                idx,
                rect: r,
                is_dir: false,
                header: false,
            });
        }
    }
}

/// Classic squarified treemap: rows of items are laid along the shortest side
/// of the remaining rectangle while the worst aspect ratio keeps improving.
fn squarify(items: &[(u32, f32)], rect: egui::Rect) -> Vec<(u32, egui::Rect)> {
    let mut result = Vec::with_capacity(items.len());
    let total: f32 = items.iter().map(|(_, w)| w).sum();
    if total <= 0.0 || rect.width() <= 1.0 || rect.height() <= 1.0 {
        return result;
    }
    let area = rect.width() * rect.height();
    let mut scaled: Vec<(u32, f32)> = items
        .iter()
        .map(|(idx, w)| (*idx, (w * area / total).max(0.001)))
        .collect();
    scaled.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut remaining = rect;
    let mut i = 0usize;
    while i < scaled.len() && remaining.width() > 0.5 && remaining.height() > 0.5 {
        let side = remaining.width().min(remaining.height()).max(1e-6);
        let mut row: Vec<(u32, f32)> = Vec::new();
        let mut row_sum = 0.0f32;
        let mut row_max = 0.0f32;
        let mut row_min = f32::INFINITY;
        let mut worst = f32::INFINITY;

        while i < scaled.len() {
            let a = scaled[i].1;
            let new_sum = row_sum + a;
            let new_max = a.max(row_max);
            let new_min = a.min(row_min);
            let ratio = worst_ratio(new_sum, new_max, new_min, side);
            if ratio <= worst {
                row.push(scaled[i]);
                row_sum = new_sum;
                row_max = new_max;
                row_min = new_min;
                worst = ratio;
                i += 1;
            } else {
                break;
            }
        }
        if row_sum <= 0.0 {
            i += 1;
            continue;
        }

        if remaining.width() < remaining.height() {
            let band = (row_sum / remaining.width()).min(remaining.height());
            let mut x = remaining.min.x;
            for (idx, a) in &row {
                let w = if band > 0.0 { a / band } else { 0.0 };
                let r = egui::Rect::from_min_max(
                    egui::pos2(x, remaining.min.y),
                    egui::pos2(x + w, remaining.min.y + band),
                );
                result.push((*idx, r.intersect(rect)));
                x += w;
            }
            remaining.min.y += band;
        } else {
            let band = (row_sum / remaining.height()).min(remaining.width());
            let mut y = remaining.min.y;
            for (idx, a) in &row {
                let h = if band > 0.0 { a / band } else { 0.0 };
                let r = egui::Rect::from_min_max(
                    egui::pos2(remaining.min.x, y),
                    egui::pos2(remaining.min.x + band, y + h),
                );
                result.push((*idx, r.intersect(rect)));
                y += h;
            }
            remaining.min.x += band;
        }
    }
    result
}

fn worst_ratio(sum: f32, rmax: f32, rmin: f32, side: f32) -> f32 {
    if sum <= 0.0 || rmin <= 0.0 || rmin == f32::INFINITY || side <= 0.0 {
        return f32::INFINITY;
    }
    let s2 = sum * sum;
    let w2 = side * side;
    ((w2 * rmax) / s2).max(s2 / (w2 * rmin))
}

/// Deepest placed entry containing `pos` (children are emitted after parents).
pub fn hit_test(placed: &[PlacedRect], pos: egui::Pos2) -> Option<&PlacedRect> {
    placed.iter().rev().find(|p| p.rect.contains(pos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::disk_analysis_model::DiskAnalysisModel;
    use mtt_search_protocol::{DiskAnalysisRecord, DiskAnalysisSnapshot};

    fn model_with_children(sizes: &[u64]) -> DiskAnalysisModel {
        let mut records = vec![DiskAnalysisRecord {
            frn: 5,
            parent_frn: 5,
            name: String::new(),
            size: 0,
            is_dir: true,
            is_reparse: false,
        }];
        for (i, size) in sizes.iter().enumerate() {
            records.push(DiskAnalysisRecord {
                frn: 100 + i as u64,
                parent_frn: 5,
                name: format!("f{i}.bin"),
                size: *size,
                is_dir: false,
                is_reparse: false,
            });
        }
        DiskAnalysisModel::build(DiskAnalysisSnapshot {
            drive_letter: 'C',
            records,
        })
    }

    #[test]
    fn layout_covers_area_without_overlap() {
        let model = model_with_children(&[100, 300, 600, 50, 250]);
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
        let placed = layout(&model, model.root, area);
        // root + 5 leaves
        assert_eq!(placed.len(), 5);

        let total: f32 = placed
            .iter()
            .map(|p| p.rect.width() * p.rect.height())
            .sum();
        let area_total = area.width() * area.height();
        assert!((total - area_total).abs() / area_total < 0.02);

        // Pairwise overlap check (allow 1px tolerance for float drift).
        for a in 0..placed.len() {
            for b in (a + 1)..placed.len() {
                let ra = placed[a].rect.shrink(1.0);
                let rb = placed[b].rect;
                assert!(ra.intersect(rb).width() <= 0.0 || ra.intersect(rb).height() <= 0.0);
            }
        }
    }

    #[test]
    fn areas_proportional_to_weights() {
        let model = model_with_children(&[100, 300]);
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        let placed = layout(&model, model.root, area);
        assert_eq!(placed.len(), 2);
        let a0 = placed[0].rect.width() * placed[0].rect.height();
        let a1 = placed[1].rect.width() * placed[1].rect.height();
        let (big, small) = if a0 > a1 { (a0, a1) } else { (a1, a0) };
        let ratio = big / small;
        assert!((ratio - 3.0).abs() < 0.05, "ratio was {ratio}");
    }
}
