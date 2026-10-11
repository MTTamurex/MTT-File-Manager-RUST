use docx_layout::display_list::{DisplayList, Primitive};

use super::search::{glyph_character_bounds, text_run_bounds, SearchBounds};

struct SelectedRun {
    text: String,
    paragraph: Option<String>,
    line_index: Option<u64>,
    top: f32,
    left: f32,
}

pub(super) fn selected_text(
    display_list: &DisplayList,
    page_index: usize,
    selection: SearchBounds,
) -> String {
    let Some(page) = display_list.pages.get(page_index) else {
        return String::new();
    };

    let mut runs = Vec::new();
    collect_runs(&mut runs, &page.primitives, selection);
    for note in &page.note_areas {
        collect_runs(&mut runs, &note.separator_primitives, selection);
        collect_runs(&mut runs, &note.primitives, selection);
    }
    if let Some(header) = &page.header {
        collect_runs(&mut runs, &header.primitives, selection);
    }
    if let Some(footer) = &page.footer {
        collect_runs(&mut runs, &footer.primitives, selection);
    }

    runs.sort_by(|a, b| {
        a.top
            .total_cmp(&b.top)
            .then_with(|| a.left.total_cmp(&b.left))
    });

    let mut text = String::new();
    let mut previous_paragraph: Option<String> = None;
    let mut previous_line: Option<u64> = None;
    let mut previous_top: Option<f32> = None;
    for run in runs {
        if !text.is_empty() {
            let new_paragraph = run.paragraph != previous_paragraph;
            let new_line = match (run.line_index, previous_line) {
                (Some(current), Some(previous)) => current != previous,
                _ => previous_top.is_some_and(|top| (run.top - top).abs() > 4.0),
            };
            if new_paragraph || new_line {
                text.push('\n');
            }
        }
        text.push_str(&run.text);
        previous_paragraph = run.paragraph;
        previous_line = run.line_index;
        previous_top = Some(run.top);
    }

    text.replace("\r\n", "\n").trim().to_owned()
}

fn collect_runs(target: &mut Vec<SelectedRun>, primitives: &[Primitive], selection: SearchBounds) {
    for primitive in primitives {
        let (attrs, hidden, characters) = match primitive {
            Primitive::Text(run) => {
                let Some(bounds) = text_run_bounds(run) else {
                    continue;
                };
                let count = run.text.chars().count();
                if count == 0 {
                    continue;
                }
                let character_width = (bounds.right - bounds.left) / count as f32;
                let rtl = run.rtl.unwrap_or(false);
                let characters = run
                    .text
                    .chars()
                    .enumerate()
                    .map(|(index, character)| {
                        let visual_index = if rtl { count - index - 1 } else { index };
                        let left = bounds.left + character_width * visual_index as f32;
                        let right = if visual_index + 1 == count {
                            bounds.right
                        } else {
                            left + character_width
                        };
                        (
                            character,
                            SearchBounds {
                                left,
                                top: bounds.top,
                                right,
                                bottom: bounds.bottom,
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                (&run.attrs, run.hidden, characters)
            }
            Primitive::GlyphRun(run) => {
                let bounds = glyph_character_bounds(run);
                let characters = run
                    .text
                    .chars()
                    .zip(bounds)
                    .filter_map(|(character, bounds)| bounds.map(|bounds| (character, bounds)))
                    .collect::<Vec<_>>();
                (&run.attrs, run.hidden, characters)
            }
            _ => continue,
        };
        if hidden {
            continue;
        }

        let mut selected = String::new();
        let mut selected_bounds: Option<SearchBounds> = None;
        for (character, bounds) in characters {
            if bounds.overlaps(selection) {
                selected.push(character);
                selected_bounds = Some(selected_bounds.map_or(bounds, |current| SearchBounds {
                    left: current.left.min(bounds.left),
                    top: current.top.min(bounds.top),
                    right: current.right.max(bounds.right),
                    bottom: current.bottom.max(bounds.bottom),
                }));
            }
        }
        let Some(bounds) = selected_bounds else {
            continue;
        };
        if selected.is_empty() {
            continue;
        }

        let paragraph = attrs
            .para_id
            .as_ref()
            .map(|value| format!("p:{value}"))
            .or_else(|| attrs.block_key.as_ref().map(|value| format!("k:{value}")))
            .or_else(|| attrs.block_id.as_ref().map(|value| format!("b:{value}")));
        target.push(SelectedRun {
            text: selected,
            paragraph,
            line_index: attrs.line_index,
            top: bounds.top,
            left: bounds.left,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::selected_text;
    use crate::docx_viewer::search::SearchBounds;
    use docx_layout::display_list::DisplayList;
    use serde_json::json;

    #[test]
    fn selection_extracts_partial_text_runs_and_keeps_paragraph_breaks() {
        let display_list: DisplayList = serde_json::from_value(json!({
            "pages": [{
                "pageIndex": 0,
                "width": 100,
                "height": 100,
                "primitives": [
                    {
                        "kind": "text",
                        "text": "abcdef",
                        "x": 0,
                        "baselineY": 20,
                        "width": 60,
                        "font": "10px Arial",
                        "color": "#000000",
                        "paraId": "first"
                    },
                    {
                        "kind": "text",
                        "text": "ghij",
                        "x": 20,
                        "baselineY": 40,
                        "width": 20,
                        "font": "10px Arial",
                        "color": "#000000",
                        "paraId": "second"
                    }
                ]
            }]
        }))
        .unwrap();

        let text = selected_text(
            &display_list,
            0,
            SearchBounds::new(20.1, 10.0, 39.9, 43.0).unwrap(),
        );

        assert_eq!(text, "cd\nghij");
    }
}
