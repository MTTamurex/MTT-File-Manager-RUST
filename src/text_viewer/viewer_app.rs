//! Text viewer application built on eframe/egui.
//!
//! Features:
//! - Monospace line-numbered rendering for plain text
//! - Formatted CommonMark rendering for Markdown
//! - Word wrap toggle
//! - Text search with Ctrl+F
//! - Select all + copy (Ctrl+A, Ctrl+C)
//! - Encoding detection (UTF-8, UTF-16, and legacy text encodings)
//! - Binary file rejection
//! - Go-to-line (Ctrl+G)

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use egui_commonmark::CommonMarkCache;
use rust_i18n::t;

mod content;
mod highlighting;
mod markdown;
mod runtime;
mod search;
#[cfg(test)]
mod tests;
mod toolbar;
use highlighting::SyntaxHighlighting;

/// Font size bounds.
const MIN_FONT_SIZE: f32 = 8.0;
const MAX_FONT_SIZE: f32 = 48.0;
const DEFAULT_FONT_SIZE: f32 = 14.0;

// ── App ──────────────────────────────────────────────────────────────────────

pub struct TextViewerApp {
    /// Original file path.
    file_path: PathBuf,

    /// Full decoded file content (single contiguous buffer).
    /// Lines are accessed via [`Self::line`] using `line_offsets` instead of
    /// being stored as one `String` per line, which previously caused ~24 B
    /// of `String`-header overhead per line plus heap fragmentation.
    content: Arc<String>,

    /// Byte offset (into `content`) of the start of each line.
    /// `u32` is sufficient because [`super::MAX_TEXT_FILE_SIZE`] caps files
    /// well below 4 GiB. The end of line `i` is `line_offsets[i+1]` (or
    /// `content.len()` for the last line).
    line_offsets: Vec<u32>,

    /// Detected encoding label (for display in the toolbar).
    encoding_label: &'static str,

    /// Markdown files are rendered as formatted CommonMark instead of source rows.
    is_markdown: bool,
    markdown_cache: CommonMarkCache,
    markdown_document: markdown::MarkdownDocument,
    syntax_highlighting: SyntaxHighlighting,

    /// Current font size.
    font_size: f32,

    /// Whether word wrap is enabled.
    word_wrap: bool,

    search: search::TextSearchState,

    /// Go-to-line dialog state.
    goto_open: bool,
    goto_input: String,
    goto_focus_request: bool,

    /// Scroll target (line index to scroll to).
    scroll_to_line: Option<usize>,

    /// Whether to apply dark theme (set once, consumed on first frame).
    dark_mode: Option<bool>,

    /// Throttle for polling the shared theme preference while running.
    last_theme_poll: std::time::Instant,

    /// Native window handle (raw value) for title-bar theming updates.
    native_hwnd: Option<isize>,

    /// Total line count (cached for toolbar).
    total_lines: usize,

    /// Total file size in bytes (for toolbar display).
    file_size_bytes: u64,
    last_memory_activity: std::time::Instant,
    last_memory_trim_check: std::time::Instant,
    last_memory_trim_request: std::time::Instant,
    memory_activity_generation: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl TextViewerApp {
    pub fn new(path: PathBuf, dark_mode: bool) -> Result<Self, String> {
        // Read raw bytes
        let raw = std::fs::read(&path)
            .map_err(|e| t!("textviewer.read_failed", error = e.to_string()).to_string())?;

        let file_size_bytes = raw.len() as u64;
        let is_markdown = crate::ui::preview_panel::markdown_preview::is_markdown(&path);
        let decoded = crate::text_viewer::encoding::decode_text(&raw, false)
            .map_err(|_| t!("textviewer.binary_file").to_string())?;
        let text = if is_markdown {
            crate::ui::preview_panel::markdown_safety::strip_remote_images(&decoded.text)
        } else {
            decoded.text
        };

        // Build line offsets without copying each line into its own `String`;
        // the decoded text remains the viewer's single content buffer.
        let line_offsets = build_line_offsets(&text);
        let total_lines = line_offsets.len();
        let markdown_document = if is_markdown {
            markdown::MarkdownDocument::new(&text)
        } else {
            markdown::MarkdownDocument::default()
        };

        Ok(Self {
            file_path: path,
            content: Arc::new(text),
            line_offsets,
            encoding_label: decoded.encoding,
            is_markdown,
            markdown_cache: CommonMarkCache::default(),
            markdown_document,
            syntax_highlighting: SyntaxHighlighting::default(),
            font_size: DEFAULT_FONT_SIZE,
            word_wrap: false,
            search: search::TextSearchState::default(),
            goto_open: false,
            goto_input: String::new(),
            goto_focus_request: false,
            scroll_to_line: None,
            dark_mode: Some(dark_mode),
            last_theme_poll: std::time::Instant::now(),
            native_hwnd: None,
            total_lines,
            file_size_bytes,
            last_memory_activity: std::time::Instant::now(),
            last_memory_trim_check: std::time::Instant::now(),
            last_memory_trim_request: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or_else(std::time::Instant::now),
            memory_activity_generation: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    pub(super) fn syntax_job_for_line(&self, line_index: usize) -> Option<egui::text::LayoutJob> {
        let start = *self.line_offsets.get(line_index)? as usize;
        let end = start + self.line(line_index).len();
        self.syntax_highlighting.job_for_range(start..end)
    }

    // ── Keyboard shortcuts ───────────────────────────────────────────────

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        ctx.input(|i| {
            // Ctrl+G: toggle go-to-line
            if i.modifiers.ctrl && i.key_pressed(egui::Key::G) {
                self.toggle_goto();
            }
            if i.key_pressed(egui::Key::Escape) && self.goto_open {
                self.goto_open = false;
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
            let zoom_delta = i.zoom_delta() - 1.0;
            if i.modifiers.ctrl && zoom_delta.abs() > f32::EPSILON {
                let delta = zoom_delta.signum();
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

    fn toggle_goto(&mut self) {
        self.goto_open = !self.goto_open;
        if self.goto_open {
            self.goto_focus_request = true;
            self.close_search();
        }
    }

    fn run_idle_working_set_trim(&mut self, ctx: &egui::Context) {
        let now = std::time::Instant::now();
        if ctx.input(|input| !input.raw.events.is_empty()) {
            self.last_memory_activity = now;
            self.memory_activity_generation
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        }

        let trim_check_interval = crate::image_viewer::metrics::working_set_trim_check_interval();
        let since_last_check = now.duration_since(self.last_memory_trim_check);
        if since_last_check < trim_check_interval {
            ctx.request_repaint_after(trim_check_interval - since_last_check);
            return;
        }
        self.last_memory_trim_check = now;

        let working_set_bytes = crate::image_viewer::metrics::current_working_set_bytes();
        if !crate::image_viewer::metrics::idle_trim_is_due(
            now,
            self.last_memory_activity,
            self.last_memory_trim_request,
            working_set_bytes,
            false,
        ) {
            if working_set_bytes >= crate::image_viewer::metrics::working_set_trim_min_bytes() {
                let wait = crate::image_viewer::metrics::idle_trim_wait_remaining(
                    now,
                    self.last_memory_activity,
                    self.last_memory_trim_request,
                );
                ctx.request_repaint_after(wait.max(trim_check_interval));
            }
            return;
        }

        let generation = self
            .memory_activity_generation
            .load(std::sync::atomic::Ordering::Acquire);
        if crate::image_viewer::metrics::request_idle_working_set_trim(
            std::sync::Arc::clone(&self.memory_activity_generation),
            generation,
        ) {
            self.last_memory_trim_request = now;
            let wait = crate::image_viewer::metrics::idle_trim_wait_remaining(
                now,
                self.last_memory_activity,
                now,
            );
            ctx.request_repaint_after(wait.max(trim_check_interval));
        }
    }

    /// Returns the text of line `idx` (without the trailing `\n`/`\r`).
    /// Mimics the semantics of [`str::lines`] without allocating per line.
    fn line(&self, idx: usize) -> &str {
        let start = self.line_offsets[idx] as usize;
        let end = self
            .line_offsets
            .get(idx + 1)
            .map(|&o| o as usize)
            .unwrap_or(self.content.len());
        let mut s = &self.content[start..end];
        if s.ends_with('\n') {
            s = &s[..s.len() - 1];
        }
        if s.ends_with('\r') {
            s = &s[..s.len() - 1];
        }
        s
    }
}

/// Build a vector of byte offsets pointing to the start of each line in
/// `content`. The number of entries equals the number of lines yielded by
/// [`str::lines`] for the same input (so a trailing `\n` does not produce an
/// extra empty line).
fn build_line_offsets(content: &str) -> Vec<u32> {
    let mut offsets: Vec<u32> = Vec::new();
    if content.is_empty() {
        return offsets;
    }
    offsets.push(0);
    let bytes = content.as_bytes();
    let len = bytes.len();
    for (i, &b) in bytes.iter().enumerate() {
        // Don't push an offset for a trailing newline (matches str::lines).
        if b == b'\n' && i + 1 < len {
            offsets.push((i + 1) as u32);
        }
    }
    offsets
}

// ── Error fallback app ───────────────────────────────────────────────────────

pub(super) struct ErrorApp {
    pub message: String,
}

impl eframe::App for ErrorApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::viewer_runtime::poll_viewer_fonts(ctx);
        // Reveal window (started hidden to avoid wgpu init flicker).
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| {
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
