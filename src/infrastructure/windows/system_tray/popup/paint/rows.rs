use super::super::State;
use super::{bitmap, measure, rounded, text};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::*;

pub(super) fn paint(dc: HDC, state: &State, index: usize, y: i32, title: &str) {
    let l = &state.layout;
    let r = &state.resources;
    let c = &r.colors;
    let row = &state.model.rows[index];
    let rect = |x, y, width, height| RECT {
        left: l.px(x),
        top: l.px(y),
        right: l.px(x + width),
        bottom: l.px(y + height),
    };
    let selected = state.model.selected == Some(index) && row.selectable();
    if selected {
        rounded(
            dc,
            rect(8, y + 2, l.width - 16, row.height - 4),
            l.px(8),
            c.hover,
        );
    }
    let color = if selected && c.hover == c.accent {
        c.accent_text
    } else {
        c.text
    };
    let muted = if selected && c.hover == c.accent {
        color
    } else {
        c.muted
    };
    if row.instance().is_some() {
        // Keep the timestamp beside the title while reserving the close button.
        let heading = rect(12, y + 5, l.width - 56, 22);
        text(dc, title, heading, r.bold.0, color, DT_LEFT);
        if let Some(time) = &row.sent_at {
            let previous = unsafe { SelectObject(dc, r.bold.0.into()) };
            let name_width = measure(dc, title);
            unsafe {
                SelectObject(dc, previous);
            }
            let metadata = RECT {
                left: heading.left + name_width + l.px(6),
                ..heading
            };
            if metadata.left < metadata.right {
                text(
                    dc,
                    &format!("— {} {time}", rust_i18n::t!("tray.hidden_at")),
                    metadata,
                    r.font.0,
                    muted,
                    DT_LEFT,
                );
            }
        }
        if let Some(icon) = row.icon.and_then(|icon| r.icons.get(icon)) {
            bitmap(dc, icon, rect(12, y + 28, 18, 18), r.icon_size);
        }
        let right = if row.expanded.is_some() { 64 } else { 44 };
        text(
            dc,
            &row.subtitle,
            rect(40, y + 27, l.width - 40 - right, 18),
            r.font.0,
            muted,
            DT_LEFT,
        );
        if let Some(expanded) = row.expanded {
            text(
                dc,
                if expanded { "⌄" } else { "›" },
                rect(l.width - 60, y + 27, 20, 18),
                r.bold.0,
                color,
                DT_CENTER,
            );
        }
        let close = rect(l.width - 34, y + 4, 26, row.height - 8);
        let close_color = if state.hot_close == Some(index) {
            rounded(dc, close, l.px(6), c.accent);
            c.accent_text
        } else {
            color
        };
        text(dc, "×", close, r.bold.0, close_color, DT_CENTER);
    } else {
        let x = 12 + row.indent * 12;
        if let Some(icon) = row.icon.and_then(|icon| r.icons.get(icon)) {
            bitmap(
                dc,
                icon,
                rect(x, y + (row.height - 18) / 2, 18, 18),
                r.icon_size,
            );
        }
        let title_x = if row.icon.is_some() { x + 28 } else { x };
        text(
            dc,
            title,
            rect(title_x, y, l.width - title_x - 16, row.height),
            r.font.0,
            if row.selectable() { color } else { muted },
            DT_LEFT,
        );
    }
}
