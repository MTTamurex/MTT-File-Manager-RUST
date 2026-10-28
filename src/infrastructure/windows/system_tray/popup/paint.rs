use super::super::{graphics::Bitmap, labels};
use super::{
    layout::{FOOTER, HEADER},
    State,
};
use windows::Win32::Foundation::{COLORREF, RECT, SIZE};
use windows::Win32::Graphics::Gdi::*;

mod rows;

fn fill(dc: HDC, rect: RECT, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        if !brush.is_invalid() {
            FillRect(dc, &rect, brush);
            let _ = DeleteObject(brush.into());
        }
    }
}

fn measure(dc: HDC, value: &str) -> i32 {
    let wide: Vec<u16> = value.encode_utf16().collect();
    let mut size = SIZE::default();
    if unsafe { GetTextExtentPoint32W(dc, &wide, &mut size) }.as_bool() {
        size.cx
    } else {
        wide.len() as i32 * 24
    }
}

fn text(
    dc: HDC,
    value: &str,
    mut rect: RECT,
    font: HFONT,
    color: COLORREF,
    alignment: DRAW_TEXT_FORMAT,
) {
    unsafe {
        let previous = SelectObject(dc, font.into());
        let value = value.replace(['\r', '\n', '\t', '\0'], " ");
        let compact = labels::compact(&value, (rect.right - rect.left).max(0), &|value| {
            measure(dc, value)
        });
        let mut wide: Vec<u16> = compact.encode_utf16().collect();
        SetTextColor(dc, color);
        DrawTextW(
            dc,
            &mut wide,
            &mut rect,
            DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX | alignment,
        );
        SelectObject(dc, previous);
    }
}

fn row_titles(dc: HDC, state: &State) -> Vec<String> {
    let mut titles: Vec<_> = state
        .model
        .rows
        .iter()
        .map(|row| row.title.clone())
        .collect();
    let previous = unsafe { SelectObject(dc, state.resources.font.0.into()) };
    for indent in [1, 2] {
        let group: Vec<_> = state
            .model
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.indent == indent && row.icon.is_some())
            .map(|(index, row)| (index, row.title.as_str()))
            .collect();
        let values: Vec<_> = group.iter().map(|(_, title)| *title).collect();
        let budget = state
            .layout
            .px(state.layout.width - (12 + indent * 12 + 28) - 16)
            - 1;
        let labels = labels::siblings(&values, budget, &|value| measure(dc, value));
        for ((index, _), label) in group.into_iter().zip(labels) {
            titles[index] = label;
        }
    }
    unsafe {
        SelectObject(dc, previous);
    }
    titles
}

fn rounded(dc: HDC, rect: RECT, radius: i32, color: COLORREF) {
    unsafe {
        let brush = CreateSolidBrush(color);
        if brush.is_invalid() {
            return;
        }
        let previous_brush = SelectObject(dc, brush.into());
        let previous_pen = SelectObject(dc, GetStockObject(NULL_PEN));
        let _ = RoundRect(
            dc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(dc, previous_pen);
        SelectObject(dc, previous_brush);
        let _ = DeleteObject(brush.into());
    }
}

fn bitmap(dc: HDC, bitmap: HBITMAP, rect: RECT, source_size: i32) {
    unsafe {
        let source = CreateCompatibleDC(Some(dc));
        if source.is_invalid() {
            return;
        }
        let previous = SelectObject(source, bitmap.into());
        let _ = AlphaBlend(
            dc,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            source,
            0,
            0,
            source_size,
            source_size,
            BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            },
        );
        SelectObject(source, previous);
        let _ = DeleteDC(source);
    }
}

pub(super) fn paint(dc: HDC, state: &State) {
    let l = &state.layout;
    let r = &state.resources;
    let c = &r.colors;
    let rect = |x, y, width, height| RECT {
        left: l.px(x),
        top: l.px(y),
        right: l.px(x + width),
        bottom: l.px(y + height),
    };
    let width = l.width;
    fill(dc, rect(0, 0, width, l.height), c.background);
    unsafe {
        SetBkMode(dc, TRANSPARENT);
    }
    // The system clips this surface to rounded corners; keep a subtle outline.
    fill(dc, rect(0, 0, width, 1), c.border);
    fill(dc, rect(0, l.height - 1, width, 1), c.border);
    fill(dc, rect(0, 0, 1, l.height), c.border);
    fill(dc, rect(width - 1, 0, 1, l.height), c.border);
    if let Some(app) = &r.app {
        bitmap(dc, app.0, rect(12, 12, 28, 28), r.icon_size);
    }
    text(
        dc,
        "MTT File Manager",
        rect(50, 6, width - 114, 40),
        r.bold.0,
        c.text,
        DT_LEFT,
    );
    rounded(dc, rect(width - 48, 14, 32, 24), l.px(16), c.accent);
    text(
        dc,
        &state.model.windows.len().to_string(),
        rect(width - 46, 14, 28, 24),
        r.font.0,
        c.accent_text,
        DT_CENTER,
    );
    fill(dc, rect(12, HEADER - 1, width - 24, 1), c.border);
    let titles = row_titles(dc, state);
    let saved = unsafe { SaveDC(dc) };
    if saved == 0 {
        return;
    }
    unsafe {
        IntersectClipRect(
            dc,
            l.px(1),
            l.px(HEADER),
            l.px(width - 1),
            l.px(l.height - FOOTER),
        );
    }
    let mut y = HEADER - state.model.scroll;
    for (index, row) in state.model.rows.iter().enumerate() {
        if y + row.height > HEADER && y < l.height - FOOTER {
            rows::paint(dc, state, index, y, &titles[index]);
        }
        y += row.height;
    }
    // A proportional scroll indicator makes long expanded lists discoverable.
    if state.model.content_height() > l.viewport() {
        let track = l.viewport();
        let thumb = (track * track / state.model.content_height())
            .max(18)
            .min(track);
        let offset = (track - thumb) * state.model.scroll / (state.model.content_height() - track);
        fill(dc, rect(width - 5, HEADER + offset, 3, thumb), c.muted);
    }
    unsafe {
        let _ = RestoreDC(dc, saved);
    }
    let footer_y = l.height - FOOTER;
    fill(dc, rect(12, footer_y, width - 24, 1), c.border);
    let selected = state.model.selected == Some(state.model.rows.len());
    if selected {
        rounded(
            dc,
            rect(8, footer_y + 5, width - 16, FOOTER - 12),
            l.px(8),
            c.hover,
        );
    }
    let color = if selected && c.hover == c.accent {
        c.accent_text
    } else {
        c.text
    };
    text(
        dc,
        "×",
        rect(14, footer_y, 20, FOOTER),
        r.bold.0,
        color,
        DT_CENTER,
    );
    text(
        dc,
        &rust_i18n::t!("tray.exit"),
        rect(50, footer_y, width - 66, FOOTER),
        r.font.0,
        color,
        DT_LEFT,
    );
}

pub(super) fn repaint(hwnd: windows::Win32::Foundation::HWND, state: &State) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let target = BeginPaint(hwnd, &mut ps);
        if target.is_invalid() {
            let _ = EndPaint(hwnd, &ps);
            return;
        }
        let memory = CreateCompatibleDC(Some(target));
        let image = Bitmap(CreateCompatibleBitmap(
            target,
            state.layout.bounds.right - state.layout.bounds.left,
            state.layout.bounds.bottom - state.layout.bounds.top,
        ));
        if memory.is_invalid() || image.0.is_invalid() {
            paint(target, state);
        } else {
            let previous = SelectObject(memory, image.0.into());
            paint(memory, state);
            let _ = BitBlt(
                target,
                0,
                0,
                state.layout.bounds.right - state.layout.bounds.left,
                state.layout.bounds.bottom - state.layout.bounds.top,
                Some(memory),
                0,
                0,
                SRCCOPY,
            );
            SelectObject(memory, previous);
        }
        if !memory.is_invalid() {
            let _ = DeleteDC(memory);
        }
        let _ = EndPaint(hwnd, &ps);
    }
}
