use super::*;
use std::io::{self, Write};

fn html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn single_line(text: &str) -> String {
    text.replace(['\r', '\n', '\t'], " ")
}

/// Quoting alone does not stop spreadsheet applications from executing formulas.
fn delimited(text: &str) -> String {
    let dangerous =
        text.starts_with(['\t', '\r', '\n']) || text.trim_start().starts_with(['=', '+', '-', '@']);
    let prefix = if dangerous { "'" } else { "" };
    format!("\"{prefix}{}\"", text.replace('"', "\"\""))
}

fn write_line(
    out: &mut impl Write,
    cells: &[String],
    separator: &str,
    spreadsheet: bool,
) -> io::Result<()> {
    let cells: Vec<_> = cells
        .iter()
        .map(|cell| {
            if spreadsheet {
                delimited(cell)
            } else {
                single_line(cell)
            }
        })
        .collect();
    writeln!(out, "{}\r", cells.join(separator))
}

fn write_table_line(out: &mut impl Write, cells: &[String], widths: &[usize]) -> io::Result<()> {
    for (index, cell) in cells.iter().enumerate() {
        let cell = single_line(cell);
        write!(out, "{cell}")?;
        if index + 1 < cells.len() {
            let padding = widths[index].saturating_sub(cell.chars().count()) + 2;
            write!(out, "{}", " ".repeat(padding))?;
        }
    }
    out.write_all(b"\r\n")
}

pub(super) fn render(
    out: &mut impl Write,
    options: &Options,
    listing: &Listing,
    progress: &Progress,
) -> Result<(), ExportError> {
    render_inner(out, options, listing, progress).map_err(|error| {
        if progress.cancelled.load(Ordering::Relaxed) {
            ExportError::Cancelled
        } else if error.kind() == io::ErrorKind::FileTooLarge {
            ExportError::TooLarge
        } else {
            ExportError::Write
        }
    })
}

fn render_inner(
    out: &mut impl Write,
    options: &Options,
    listing: &Listing,
    progress: &Progress,
) -> io::Result<()> {
    let columns: Vec<_> = options
        .columns
        .iter()
        .filter(|column| column.enabled)
        .collect();
    let headers: Vec<_> = columns.iter().map(|column| column.label.clone()).collect();
    let text_tree = options.format == Format::Text && options.recursive && options.tree;
    let name_column = columns
        .iter()
        .position(|column| column.field == Field::Column(ColumnId::Name));
    let mut widths: Vec<_> = headers
        .iter()
        .map(|header| header.chars().count())
        .collect();
    if options.format == Format::Text {
        for row in &listing.rows {
            if progress.check().is_err() {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            for (index, cell) in row.cells.iter().enumerate() {
                let indent = if text_tree && name_column == Some(index) {
                    row.depth.saturating_mul(2)
                } else {
                    0
                };
                widths[index] = widths[index].max(cell.chars().count().saturating_add(indent));
            }
        }
    }
    if options.format == Format::Html {
        writeln!(out, "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>{}</title><style>body{{font:14px system-ui;margin:2rem;color:#222;background:#fff}}.row{{display:grid;grid-template-columns:repeat({},minmax(8rem,1fr));gap:1rem;padding:.35rem;border-bottom:1px solid #ddd}}.row span{{white-space:pre-wrap;overflow-wrap:anywhere}}.head{{font-weight:bold}}.children{{margin-left:1.5rem}}summary{{cursor:pointer}}summary .row{{display:inline-grid;width:95%}}</style></head><body>", html(&rust_i18n::t!("export_listing.title")), columns.len().max(1))?;
        writeln!(
            out,
            "<div class=\"row head\">{}</div>",
            headers
                .iter()
                .map(|cell| format!("<span>{}</span>", html(cell)))
                .collect::<String>()
        )?;
    } else {
        // BOM makes UTF-8 CSV/TSV open correctly in Windows Excel.
        if matches!(options.format, Format::Csv | Format::Tsv) {
            out.write_all(b"\xef\xbb\xbf")?;
        }
        if options.format == Format::Text {
            write_table_line(out, &headers, &widths)?;
        } else {
            write_line(
                out,
                &headers,
                if options.format == Format::Csv {
                    options.csv_separator.text()
                } else {
                    "\t"
                },
                true,
            )?;
        }
    }
    let mut open_directories = Vec::new();
    for (index, row) in listing.rows.iter().enumerate() {
        if progress.check().is_err() {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        if options.format == Format::Html {
            while open_directories
                .last()
                .is_some_and(|depth| *depth >= row.depth)
            {
                out.write_all(b"</div></details>\n")?;
                open_directories.pop();
            }
        }
        let tree = options.recursive
            && (options.format == Format::Html || (options.format == Format::Text && options.tree));
        let has_children = listing
            .rows
            .get(index + 1)
            .is_some_and(|next| next.depth > row.depth);
        // Excluded directories may remain as structural parents, never as table records.
        if !(row.included || tree && row.directory && has_children) {
            continue;
        }
        match options.format {
            Format::Csv | Format::Tsv => write_line(
                out,
                &row.cells,
                if options.format == Format::Csv {
                    options.csv_separator.text()
                } else {
                    "\t"
                },
                true,
            )?,
            Format::Text => {
                let mut cells = row.cells.clone();
                if tree {
                    if let Some(index) = name_column {
                        cells[index] =
                            format!("{}{}", "  ".repeat(row.depth), single_line(&cells[index]));
                    }
                }
                write_table_line(out, &cells, &widths)?;
            }
            Format::Html => {
                let cells = row
                    .cells
                    .iter()
                    .map(|cell| format!("<span>{}</span>", html(cell)))
                    .collect::<String>();
                if tree && row.directory {
                    // Always identify the folder even when the name column is disabled.
                    writeln!(out, "<details open><summary><span class=\"row\" aria-label=\"{}\">{cells}</span></summary><div class=\"children\">", html(&row.name))?;
                    open_directories.push(row.depth);
                } else {
                    writeln!(out, "<div class=\"row\">{cells}</div>")?;
                }
            }
        }
    }
    if options.format == Format::Html {
        for _ in open_directories {
            out.write_all(b"</div></details>\n")?;
        }
        out.write_all(b"</body></html>\n")?;
    }
    Ok(())
}
