use super::*;
use crate::docx_viewer::search::{DocxSearchIndex, SearchRequest};

#[test]
fn small_caps_docx_renders_and_preserves_search_and_selection() {
    let source = tempfile::NamedTempFile::new().unwrap();
    let bytes = synthetic_docx_with_text_run(
        r#"<w:r><w:rPr><w:smallCaps/></w:rPr><w:t>Mixed Helloße</w:t></w:r>
        <w:r><w:rPr><w:b/><w:i/><w:smallCaps/></w:rPr><w:t xml:space="preserve"> formatted</w:t></w:r>
        <w:r><w:t xml:space="preserve"> normal</w:t></w:r>"#,
    );
    std::fs::write(source.path(), &bytes).unwrap();
    let mut renderer = DocxRenderer::open(source.path()).unwrap();
    let before = renderer.search_display_list();
    // Authoritative layout supplies the original cluster origins. Rendering
    // must change the glyph appearance, not recalculate wrapping or positions.
    assert!(before.pages[0].primitives.iter().any(|primitive| {
        matches!(primitive, docx_layout::display_list::Primitive::Text(run) if run.small_caps)
    }));
    let page = renderer.render_page(0).unwrap();
    assert!(page
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| { pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128 }));
    assert_eq!(*renderer.search_display_list(), *before);
    let (width, height) = renderer.page_sizes()[0];
    let selection = SearchBounds::new(0.0, 0.0, width, height).unwrap();
    assert_eq!(
        renderer.selected_text(0, selection),
        "Mixed Helloße formatted normal",
    );
    let index = DocxSearchIndex::new(renderer.search_display_list());
    let (request_tx, request_rx) = crossbeam_channel::unbounded();
    let (result_tx, result_rx) = crossbeam_channel::unbounded();
    request_tx
        .send(SearchRequest {
            query: "Helloße".into(),
            generation: 0,
        })
        .unwrap();
    drop(request_tx);
    crate::docx_viewer::search::run_search_worker(
        index,
        request_rx,
        result_tx,
        eframe::egui::Context::default(),
    );
    let result = result_rx.recv().unwrap();
    assert_eq!(result.matches.len(), 1);
    assert_eq!(std::fs::read(source.path()).unwrap(), bytes);
}

#[test]
fn small_caps_language_and_all_caps_docx_render() {
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        source.path(),
        synthetic_docx_with_text_run(
            r#"<w:r><w:rPr><w:smallCaps/><w:lang w:val="tr-TR"/></w:rPr><w:t>iıß</w:t></w:r>
            <w:r><w:rPr><w:smallCaps/><w:caps/></w:rPr><w:t xml:space="preserve"> Mixed</w:t></w:r>"#,
        ),
    ).unwrap();
    let mut renderer = DocxRenderer::open(source.path()).unwrap();
    let page = renderer.render_page(0).unwrap();
    assert!(page
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| { pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128 }));
}
