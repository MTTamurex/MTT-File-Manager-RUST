//! Text viewer application built on eframe/egui.
//!
//! Features:
//! - Monospace font rendering with line numbers
//! - Word wrap toggle
//! - Text search with Ctrl+F
//! - Select all + copy (Ctrl+A, Ctrl+C)
//! - Encoding detection (UTF-8 with BOM / UTF-8 / Latin-1 fallback)
//! - Binary file rejection
//! - Go-to-line (Ctrl+G)

use std::path::PathBuf;

use eframe::egui;
use rust_i18n::t;

/// Maximum number of null bytes allowed in the first 8 KB before we consider
/// the file binary (and reject it).
const MAX_NULL_BYTES_SAMPLE: usize = 4;

/// Size of the sample read for binary detection (8 KB).
const BINARY_DETECTION_SIZE: usize = 8 * 1024;

/// Font size bounds.
const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 48.0;
const DEFAULT_FONT_SIZE: f32 = 14.0;

// ── App ──────────────────────────────────────────────────────────────────────

pub struct TextViewerApp {
    /// Original file path.
    file_path: PathBuf,

    /// All lines of the file (split on `\n`; `\r` stripped).
    lines: Vec<String>,

    /// Detected encoding label (for display in the toolbar).
    encoding_label: &'static str,

    /// Current font size.
    font_size: f32,

    /// Whether word wrap is enabled.
    word_wrap: bool,

    /// Search query (visible when search bar is open).
    search_open: bool,
    search_query: String,
    /// Indices of lines matching the current search query.
    search_hits: Vec<usize>,
    /// Currently focused hit index.
    search_hit_cursor: usize,
    /// Whether the search field should grab focus this frame.
    search_focus_request: bool,

    /// Go-to-line dialog state.
    goto_open: bool,
    goto_input: String,
    goto_focus_request: bool,

    /// Scroll target (line index to scroll to).
    scroll_to_line: Option<usize>,

    /// Whether to apply dark theme (set once, consumed on first frame).
    dark_mode: Option<bool>,

    /// Total line count (cached for toolbar).
    total_lines: usize,

    /// Total file size in bytes (for toolbar display).
    file_size_bytes: u64,
}

impl TextViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Result<Self, String> {
        // Read raw bytes
        let raw = std::fs::read(&path).map_err(|e| {
            t!("textviewer.read_failed", error = e.to_string()).to_string()
        })?;

        let file_size_bytes = raw.len() as u64;

        // Binary detection: check first 8 KB for null bytes
        let sample_len = raw.len().min(BINARY_DETECTION_SIZE);
        let null_count = raw[..sample_len].iter().filter(|&&b| b == 0).count();
        if null_count > MAX_NULL_BYTES_SAMPLE {
            return Err(t!("textviewer.binary_file").to_string());
        }

        // Encoding detection
        let (text, encoding_label) = decode_text(&raw);

        // Split into lines
        let lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        let total_lines = lines.len();

        Ok(Self {
            file_path: path,
            lines,
            encoding_label,
            font_size: DEFAULT_FONT_SIZE,
            word_wrap: false,
            search_open: false,
            search_query: String::new(),
            search_hits: Vec::new(),
            search_hit_cursor: 0,
            search_focus_request: false,
            goto_open: false,
            goto_input: String::new(),
            goto_focus_request: false,
            scroll_to_line: None,
            dark_mode: Some(dark_mode),
            total_lines,
            file_size_bytes,
        })
    }

    // ── Toolbar ──────────────────────────────────────────────────────────

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;

            // File info
            let file_name = self
                .file_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            ui.label(
                egui::RichText::new(&file_name)
                    .strong()
                    .size(13.0),
            );

            ui.separator();

            // Line count
            ui.label(
                egui::RichText::new(t!(
                    "textviewer.line_count",
                    count = self.total_lines.to_string()
                ).to_string())
                .size(12.0),
            );

            ui.separator();

            // File size
            ui.label(
                egui::RichText::new(format_file_size(self.file_size_bytes))
                    .size(12.0),
            );

            ui.separator();

            // Encoding
            ui.label(
                egui::RichText::new(self.encoding_label)
                    .size(12.0),
            );

            ui.separator();

            // Font size controls
            if ui
                .button(egui::RichText::new("A−").size(13.0))
                .on_hover_text(t!("textviewer.font_decrease"))
                .clicked()
            {
                self.font_size = (self.font_size - 1.0).max(MIN_FONT_SIZE);
            }
            ui.label(
                egui::RichText::new(format!("{:.0}px", self.font_size))
                    .size(12.0)
                    .monospace(),
            );
            if ui
                .button(egui::RichText::new("A+").size(13.0))
                .on_hover_text(t!("textviewer.font_increase"))
                .clicked()
            {
                self.font_size = (self.font_size + 1.0).min(MAX_FONT_SIZE);
            }

            ui.separator();

            // Word wrap toggle
            let wrap_label = if self.word_wrap {
                t!("textviewer.wrap_on")
            } else {
                t!("textviewer.wrap_off")
            };
            if ui
                .selectable_label(self.word_wrap, wrap_label.to_string())
                .on_hover_text(t!("textviewer.wrap_toggle"))
                .clicked()
            {
                self.word_wrap = !self.word_wrap;
            }

            ui.separator();

            // Search button
            if ui
                .button(egui::RichText::new("Find").size(12.0))
                .on_hover_text(t!("textviewer.search_hint"))
                .clicked()
            {
                self.toggle_search();
            }

            // Go-to-line button
            if ui
                .button(egui::RichText::new("Ln").size(12.0))
                .on_hover_text(t!("textviewer.goto_hint"))
                .clicked()
            {
                self.toggle_goto();
            }
        });
    }

    // ── Search bar ───────────────────────────────────────────────────────

    fn show_search_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(egui::RichText::new("Find:").strong().size(12.0));

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.search_query)
                    .desired_width(250.0)
                    .hint_text(t!("textviewer.search_placeholder").to_string()),
            );

            if self.search_focus_request {
                resp.request_focus();
                self.search_focus_request = false;
            }

            if resp.changed() {
                self.update_search_hits();
            }

            // Navigate hits
            let has_hits = !self.search_hits.is_empty();
            if has_hits {
                ui.label(format!(
                    "{}/{}",
                    if self.search_hits.is_empty() {
                        0
                    } else {
                        self.search_hit_cursor + 1
                    },
                    self.search_hits.len()
                ));
            } else if !self.search_query.is_empty() {
                ui.label(t!("textviewer.no_results").to_string());
            }

            if ui.button("<").on_hover_text(t!("textviewer.search_prev")).clicked() && has_hits {
                if self.search_hit_cursor == 0 {
                    self.search_hit_cursor = self.search_hits.len().saturating_sub(1);
                } else {
                    self.search_hit_cursor -= 1;
                }
                self.scroll_to_line = Some(self.search_hits[self.search_hit_cursor]);
            }
            if ui.button(">").on_hover_text(t!("textviewer.search_next")).clicked() && has_hits {
                self.search_hit_cursor = (self.search_hit_cursor + 1) % self.search_hits.len();
                self.scroll_to_line = Some(self.search_hits[self.search_hit_cursor]);
            }

            // Enter navigates to next hit
            if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && has_hits {
                self.search_hit_cursor = (self.search_hit_cursor + 1) % self.search_hits.len();
                self.scroll_to_line = Some(self.search_hits[self.search_hit_cursor]);
                resp.request_focus();
            }

            if ui.button("X").on_hover_text(t!("textviewer.search_close")).clicked() {
                self.search_open = false;
                self.search_query.clear();
                self.search_hits.clear();
            }
        });
    }

    // ── Go-to-line bar ───────────────────────────────────────────────────

    fn show_goto_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(t!("textviewer.goto_label").to_string());

            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.goto_input)
                    .desired_width(100.0)
                    .hint_text(format!("1–{}", self.total_lines)),
            );

            if self.goto_focus_request {
                resp.request_focus();
                self.goto_focus_request = false;
            }

            let go = ui.button("Go").clicked()
                || (resp.lost_focus()
                    && ui.input(|i| i.key_pressed(egui::Key::Enter)));

            if go {
                if let Ok(line_num) = self.goto_input.trim().parse::<usize>() {
                    if line_num >= 1 && line_num <= self.total_lines {
                        self.scroll_to_line = Some(line_num - 1);
                    }
                }
                self.goto_open = false;
            }

            if ui.button("X").clicked() {
                self.goto_open = false;
            }
        });
    }

    // ── Text content ─────────────────────────────────────────────────────

    fn show_content(&mut self, ui: &mut egui::Ui) {
        let line_number_width = format!("{}", self.total_lines).len() as f32 * self.font_size * 0.6 + 16.0;
        let available_width = ui.available_width();

        // Determine row height from font
        let row_height = self.font_size + 4.0;
        let total_rows = self.total_lines;

        // Build a set of search-hit lines for fast lookup
        let search_hit_set: std::collections::HashSet<usize> =
            self.search_hits.iter().copied().collect();
        let current_hit_line = self
            .search_hits
            .get(self.search_hit_cursor)
            .copied();

        let mut scroll_area = egui::ScrollArea::both()
            .auto_shrink([false, false]);

        // Handle scroll-to-line
        if let Some(target_line) = self.scroll_to_line.take() {
            let y = target_line as f32 * row_height;
            scroll_area = scroll_area.vertical_scroll_offset(y.max(0.0));
        }

        scroll_area.show_rows(ui, row_height, total_rows, |ui, row_range| {
            for line_idx in row_range {
                let line = &self.lines[line_idx];

                ui.horizontal(|ui| {
                    // Line number gutter
                    let line_num_text = format!("{:>width$}", line_idx + 1, width = format!("{}", self.total_lines).len());
                    ui.add_sized(
                        [line_number_width, row_height],
                        egui::Label::new(
                            egui::RichText::new(&line_num_text)
                                .monospace()
                                .size(self.font_size)
                                .color(ui.visuals().weak_text_color()),
                        ),
                    );

                    ui.separator();

                    // Highlight background for search hits
                    let is_hit = search_hit_set.contains(&line_idx);
                    let is_current = current_hit_line == Some(line_idx);

                    if is_current {
                        let rect = ui.available_rect_before_wrap();
                        let highlight_rect = egui::Rect::from_min_size(
                            rect.min,
                            egui::vec2(available_width, row_height),
                        );
                        ui.painter().rect_filled(
                            highlight_rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(255, 200, 0, 40),
                        );
                    } else if is_hit {
                        let rect = ui.available_rect_before_wrap();
                        let highlight_rect = egui::Rect::from_min_size(
                            rect.min,
                            egui::vec2(available_width, row_height),
                        );
                        ui.painter().rect_filled(
                            highlight_rect,
                            0.0,
                            egui::Color32::from_rgba_premultiplied(255, 255, 0, 20),
                        );
                    }

                    // Text content
                    let text_widget = egui::Label::new(
                        egui::RichText::new(line)
                            .monospace()
                            .size(self.font_size),
                    )
                    .selectable(true);

                    if self.word_wrap {
                        ui.add(text_widget.wrap());
                    } else {
                        ui.add(text_widget);
                    }
                });
            }
        });
    }

    // ── Keyboard shortcuts ───────────────────────────────────────────────

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        ctx.input(|i| {
            // Ctrl+F: toggle search
            if i.modifiers.ctrl && i.key_pressed(egui::Key::F) {
                self.toggle_search();
            }
            // Ctrl+G: toggle go-to-line
            if i.modifiers.ctrl && i.key_pressed(egui::Key::G) {
                self.toggle_goto();
            }
            // Escape: close search / goto
            if i.key_pressed(egui::Key::Escape) {
                if self.search_open {
                    self.search_open = false;
                    self.search_query.clear();
                    self.search_hits.clear();
                }
                if self.goto_open {
                    self.goto_open = false;
                }
            }
            // Ctrl+Plus / Ctrl+Minus: font size
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Plus) {
                self.font_size = (self.font_size + 1.0).min(MAX_FONT_SIZE);
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Minus) {
                self.font_size = (self.font_size - 1.0).max(MIN_FONT_SIZE);
            }
            // Ctrl+0: reset font size
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Num0) {
                self.font_size = DEFAULT_FONT_SIZE;
            }
            // Ctrl+scroll: font size
            if i.modifiers.ctrl && i.raw_scroll_delta.y != 0.0 {
                let delta = i.raw_scroll_delta.y.signum();
                self.font_size = (self.font_size + delta).clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
            }
            // Home / End
            if i.modifiers.ctrl && i.key_pressed(egui::Key::Home) {
                self.scroll_to_line = Some(0);
            }
            if i.modifiers.ctrl && i.key_pressed(egui::Key::End) {
                self.scroll_to_line = Some(self.total_lines.saturating_sub(1));
            }
        });
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn toggle_search(&mut self) {
        self.search_open = !self.search_open;
        if self.search_open {
            self.search_focus_request = true;
            self.goto_open = false;
        } else {
            self.search_query.clear();
            self.search_hits.clear();
        }
    }

    fn toggle_goto(&mut self) {
        self.goto_open = !self.goto_open;
        if self.goto_open {
            self.goto_focus_request = true;
            self.search_open = false;
        }
    }

    fn update_search_hits(&mut self) {
        self.search_hits.clear();
        self.search_hit_cursor = 0;

        if self.search_query.is_empty() {
            return;
        }

        let query_lower = self.search_query.to_lowercase();
        for (idx, line) in self.lines.iter().enumerate() {
            if line.to_lowercase().contains(&query_lower) {
                self.search_hits.push(idx);
            }
        }

        // Scroll to first hit
        if let Some(&first) = self.search_hits.first() {
            self.scroll_to_line = Some(first);
        }
    }
}

// ── eframe::App ──────────────────────────────────────────────────────────────

impl eframe::App for TextViewerApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Apply theme on first frame
        if let Some(dark) = self.dark_mode.take() {
            if dark {
                ctx.set_visuals(egui::Visuals::dark());
            } else {
                ctx.set_visuals(egui::Visuals::light());
            }

            use raw_window_handle::HasWindowHandle;
            if let Ok(handle) = frame.window_handle() {
                if let raw_window_handle::RawWindowHandle::Win32(wh) = handle.as_raw() {
                    let hwnd = windows::Win32::Foundation::HWND(wh.hwnd.get() as _);
                    crate::infrastructure::windows::window_corners::apply_dark_title_bar(
                        hwnd, dark,
                    );
                }
            }
        }

        self.handle_keyboard(ctx);

        // Toolbar
        egui::TopBottomPanel::top("text_toolbar").show(ctx, |ui| {
            self.show_toolbar(ui);
        });

        // Search / Goto bar (below toolbar)
        if self.search_open {
            egui::TopBottomPanel::top("text_search").show(ctx, |ui| {
                self.show_search_bar(ui);
            });
        }
        if self.goto_open {
            egui::TopBottomPanel::top("text_goto").show(ctx, |ui| {
                self.show_goto_bar(ui);
            });
        }

        // Content
        egui::CentralPanel::default().show(ctx, |ui| {
            self.show_content(ui);
        });
    }
}

// ── Error fallback app ───────────────────────────────────────────────────────

pub(super) struct ErrorApp {
    pub message: String,
}

impl eframe::App for ErrorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new(&self.message)
                        .color(egui::Color32::RED)
                        .size(16.0),
                );
            });
        });
    }
}

// ── Encoding detection ───────────────────────────────────────────────────────

/// Decode raw bytes into a String, detecting encoding:
/// 1. UTF-8 BOM → strip BOM, decode as UTF-8
/// 2. UTF-8 (valid) → use as-is
/// 3. Fallback → decode as Windows-1252 / Latin-1 (lossless for all byte values)
fn decode_text(raw: &[u8]) -> (String, &'static str) {
    // Check UTF-8 BOM
    if raw.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let without_bom = &raw[3..];
        if let Ok(s) = std::str::from_utf8(without_bom) {
            return (s.to_string(), "UTF-8 BOM");
        }
    }

    // Try UTF-8
    if let Ok(s) = std::str::from_utf8(raw) {
        return (s.to_string(), "UTF-8");
    }

    // Fallback: decode as Windows-1252 (Latin-1 superset).
    // Windows-1252 maps 0x00–0xFF to Unicode code points, so it's a lossless
    // identity mapping for 0x00–0x7F and 0xA0–0xFF, with the 0x80–0x9F range
    // mapped to specific characters.
    let text: String = raw
        .iter()
        .map(|&b| {
            // Windows-1252 → Unicode mapping for the 0x80–0x9F range
            match b {
                0x80 => '\u{20AC}', // €
                0x82 => '\u{201A}', // ‚
                0x83 => '\u{0192}', // ƒ
                0x84 => '\u{201E}', // „
                0x85 => '\u{2026}', // …
                0x86 => '\u{2020}', // †
                0x87 => '\u{2021}', // ‡
                0x88 => '\u{02C6}', // ˆ
                0x89 => '\u{2030}', // ‰
                0x8A => '\u{0160}', // Š
                0x8B => '\u{2039}', // ‹
                0x8C => '\u{0152}', // Œ
                0x8E => '\u{017D}', // Ž
                0x91 => '\u{2018}', // '
                0x92 => '\u{2019}', // '
                0x93 => '\u{201C}', // "
                0x94 => '\u{201D}', // "
                0x95 => '\u{2022}', // •
                0x96 => '\u{2013}', // –
                0x97 => '\u{2014}', // —
                0x98 => '\u{02DC}', // ˜
                0x99 => '\u{2122}', // ™
                0x9A => '\u{0161}', // š
                0x9B => '\u{203A}', // ›
                0x9C => '\u{0153}', // œ
                0x9E => '\u{017E}', // ž
                0x9F => '\u{0178}', // Ÿ
                _ => b as char,     // Identity for 0x00–0x7F, 0xA0–0xFF
            }
        })
        .collect();

    (text, "Windows-1252")
}

/// Format bytes into a human-readable file size.
fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
