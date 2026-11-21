use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::path::{Component, Path};

// Match egui_commonmark_backend 0.24's parser_options(). In particular, enabling
// math here would hide images inside "$...$" that the viewer still parses.
fn parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

/// Sanitize on the text worker, before passing any image URI to egui's loaders.
/// The build also disables image/file/HTTP loaders for this dependency.
pub(crate) fn strip_remote_images(markdown: &str) -> String {
    let mut output = String::with_capacity(markdown.len());
    let mut cursor = 0;
    let mut depth = 0;
    let mut blocked: Option<(usize, usize, String)> = None;
    for (event, range) in Parser::new_ext(markdown, parser_options()).into_offset_iter() {
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                depth += 1;
                if blocked.is_none() && !is_relative_image_target(&dest_url) {
                    blocked = Some((range.start, depth, String::new()));
                }
            }
            Event::Text(text) | Event::Code(text) => {
                if let Some((_, _, alt)) = blocked.as_mut() {
                    alt.push_str(&text);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, _, alt)) = blocked.as_mut() {
                    alt.push(' ');
                }
            }
            Event::End(TagEnd::Image) => {
                if blocked
                    .as_ref()
                    .is_some_and(|(_, start_depth, _)| *start_depth == depth)
                {
                    let (start, _, alt) = blocked.take().unwrap();
                    output.push_str(&markdown[cursor..start]);
                    // Keep surrounding Markdown tokens separated even for
                    // empty alt text (e.g. "!" + removed image + "[ref]").
                    output.push(' ');
                    for character in alt.chars() {
                        if character.is_ascii_punctuation() {
                            output.push('\\');
                        }
                        output.push(character);
                    }
                    output.push(' ');
                    cursor = range.end;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
    }
    output.push_str(&markdown[cursor..]);
    output
}

fn is_relative_image_target(target: &str) -> bool {
    let target = target.trim();
    // Reject all schemes, Windows drive-relative paths, network roots and
    // encoded URI separators that a loader might otherwise reinterpret.
    !target.is_empty()
        && !target.starts_with('/')
        && !target.contains([':', '\\', '%'])
        && !Path::new(target).components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_safe(markdown: &str) -> String {
        let sanitized = strip_remote_images(markdown);
        assert!(
            !Parser::new_ext(&sanitized, parser_options()).any(|event| {
                matches!(event, Event::Start(Tag::Image { dest_url, .. })
                if !is_relative_image_target(&dest_url))
            }),
            "{sanitized}"
        );
        sanitized
    }

    #[test]
    fn removes_remote_absolute_and_reference_images() {
        let sanitized = assert_safe(concat!(
            "![remote](https://example.org/image.png)\n",
            "![local](file:///folder/image.png)\n",
            "![relative](images/photo.png)\n",
            "![reference][remote]\n\n[remote]: https://example.org/image.png\n",
        ));
        assert!(sanitized.contains("remote"));
        assert!(sanitized.contains("![relative](images/photo.png)"));
    }

    #[test]
    fn sanitizer_and_viewer_parse_the_same_extensions() {
        assert_safe("$![image](https://example.org/image.png)$");
        assert_safe("$$\n![image](https://example.org/image.png)\n$$");
        assert_safe("| image |\n|---|\n| ![image](https://example.org/image.png) |");
    }

    #[test]
    fn nested_images_and_escaped_alt_cannot_reintroduce_uris() {
        assert_safe(
            "![outer ![inner](https://example.org/inner.png)](https://example.org/outer.png)",
        );
        assert_safe("![outer ![inner](https://example.org/inner.png)](images/photo.png)");
        assert_safe(
            r"![\!\[image\]\(https://example.org/image.png\)](https://example.org/outer.png)",
        );
        assert_safe(
            "!![](https://example.org/outer.png)[ref]\n\n[ref]: https://example.org/image.png",
        );
    }

    #[test]
    fn rejects_network_encoded_and_drive_relative_targets() {
        for target in [
            "//example.org/image.png",
            "C:image.png",
            "../image.png",
            "%2f%2fexample.org/image.png",
            r"\\server\share\image.png",
        ] {
            assert!(!is_relative_image_target(target));
        }
    }
}
