use eframe::egui;
use std::ops::Range;
use std::path::Path;
use std::sync::{Once, OnceLock};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style, Theme, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    THEMES.get_or_init(ThemeSet::load_defaults)
}

fn selected_theme(dark_mode: bool) -> Option<&'static Theme> {
    let themes = theme_set();
    themes
        .themes
        .get(if dark_mode {
            "base16-ocean.dark"
        } else {
            "base16-ocean.light"
        })
        .or_else(|| themes.themes.values().next())
}

fn selected_syntax<'a>(
    syntaxes: &'a SyntaxSet,
    path: &Path,
    text: &str,
) -> Option<&'a syntect::parsing::SyntaxReference> {
    syntaxes
        .find_syntax_by_extension(&preview_extension(path))
        .or_else(|| {
            text.lines()
                .next()
                .and_then(|line| syntaxes.find_syntax_by_first_line(line))
        })
}

pub(super) fn has_syntax(extension: &str, ctx: &egui::Context) -> bool {
    if let Some(syntaxes) = SYNTAXES.get() {
        return syntaxes.find_syntax_by_extension(extension).is_some();
    }
    // Never decompress the bundled grammar database on the UI thread.
    static START: Once = Once::new();
    START.call_once(|| {
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new()
            .name("detail-syntax-init".into())
            .spawn(move || {
                syntax_set();
                ctx.request_repaint();
            });
    });
    false
}

pub(super) fn preview_extension(path: &Path) -> String {
    path.extension()
        .and_then(|extension| extension.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .unwrap_or_default()
        .trim_start_matches('.')
        .to_ascii_lowercase()
}

pub(crate) fn build_highlight_job(
    path: &Path,
    text: &str,
    dark_mode: bool,
) -> Option<egui::text::LayoutJob> {
    let syntaxes = syntax_set();
    let theme = selected_theme(dark_mode)?;
    let syntax = selected_syntax(syntaxes, path, text)?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut job = egui::text::LayoutJob::default();
    for line in LinesWithEndings::from(text) {
        // Extremely long lines can trigger expensive grammar/regex processing.
        if line.len() > 4096 {
            return None;
        }
        for (style, chunk) in highlighter.highlight_line(line, syntaxes).ok()? {
            append_highlighted_chunk(&mut job, chunk, style, 12.0);
        }
    }
    Some(job)
}

pub(crate) fn slice_layout_job(
    job: &egui::text::LayoutJob,
    range: Range<usize>,
) -> Option<egui::text::LayoutJob> {
    let text = job.text.get(range.clone())?;
    let mut sliced = egui::text::LayoutJob::default();
    let first_section = job
        .sections
        .partition_point(|section| section.byte_range.end.0 as usize <= range.start);
    for section in &job.sections[first_section..] {
        if section.byte_range.start.0 as usize >= range.end {
            break;
        }
        let start = range.start.max(section.byte_range.start.0 as usize);
        let end = range.end.min(section.byte_range.end.0 as usize);
        if start < end {
            sliced.append(&job.text[start..end], 0.0, section.format.clone());
        }
    }
    (sliced.text == text).then_some(sliced)
}

pub(crate) fn set_layout_job_font_size(job: &mut egui::text::LayoutJob, font_size: f32) {
    for section in &mut job.sections {
        section.format.font_id.size = font_size;
    }
}

pub(crate) fn apply_syntax_colors(
    job: &mut egui::text::LayoutJob,
    syntax_job: &egui::text::LayoutJob,
) {
    if job.text != syntax_job.text {
        return;
    }

    let mut combined = egui::text::LayoutJob::default();
    let mut job_section_index = 0;
    let mut syntax_section_index = 0;

    while let (Some(job_section), Some(syntax_section)) = (
        job.sections.get(job_section_index),
        syntax_job.sections.get(syntax_section_index),
    ) {
        let start = (job_section.byte_range.start.0 as usize)
            .max(syntax_section.byte_range.start.0 as usize);
        let end =
            (job_section.byte_range.end.0 as usize).min(syntax_section.byte_range.end.0 as usize);

        if start < end {
            let mut format = job_section.format.clone();
            format.color = syntax_section.format.color;
            combined.append(&job.text[start..end], 0.0, format);
        }

        let job_end = job_section.byte_range.end.0;
        let syntax_end = syntax_section.byte_range.end.0;
        if job_end <= syntax_end {
            job_section_index += 1;
        }
        if syntax_end <= job_end {
            syntax_section_index += 1;
        }
    }

    if combined.text == job.text {
        *job = combined;
    }
}

fn append_highlighted_chunk(
    job: &mut egui::text::LayoutJob,
    chunk: &str,
    style: Style,
    font_size: f32,
) {
    job.append(
        chunk,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(font_size),
            color: egui::Color32::from_rgb(
                style.foreground.r,
                style.foreground.g,
                style.foreground.b,
            ),
            ..Default::default()
        },
    );
}

pub(super) fn plain_text_job(text: &str, dark_mode: bool) -> egui::text::LayoutJob {
    let color = if dark_mode {
        egui::Color32::from_rgb(220, 220, 220)
    } else {
        egui::Color32::from_rgb(35, 35, 35)
    };
    let mut job = egui::text::LayoutJob::default();
    job.append(
        text,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::monospace(12.0),
            color,
            ..Default::default()
        },
    );
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_job_can_be_sliced_into_styled_source_lines() {
        let source = "fn main() {\n    let answer = 42;\n}\n";
        let job = build_highlight_job(Path::new("source.rs"), source, true).unwrap();
        let start = source.find("    let answer").unwrap();
        let end = source[start..].find('\n').unwrap() + start;
        let line = slice_layout_job(&job, start..end).unwrap();

        assert_eq!(line.text, "    let answer = 42;");
        let mut colors = Vec::new();
        for section in &line.sections {
            if !colors.contains(&section.format.color) {
                colors.push(section.format.color);
            }
        }
        assert!(colors.len() > 1);
    }

    #[test]
    fn slicing_preserves_multiline_comment_state_and_unicode_crlf() {
        let source = "/* comment\r\n猫 comment\r\n*/\r\nlet value = 42;\r\n";
        let job = build_highlight_job(Path::new("source.rs"), source, true).unwrap();
        let start = source.find('猫').unwrap();
        let end = start + "猫 comment".len();
        let line = slice_layout_job(&job, start..end).unwrap();
        assert_eq!(line.text, "猫 comment");
        assert_eq!(line.sections[0].format.color, job.sections[0].format.color);
        let mut resized = line.clone();
        set_layout_job_font_size(&mut resized, 22.0);
        assert!(resized
            .sections
            .iter()
            .all(|section| section.format.font_id.size == 22.0));
        assert_eq!(
            resized.sections[0].format.color,
            line.sections[0].format.color
        );
    }
}
