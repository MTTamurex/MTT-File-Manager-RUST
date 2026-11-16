use super::*;
use docx_layout::display_list::Primitive;
use std::collections::BTreeMap;

pub(super) fn region_docx(multiple_sections: bool, notes: bool) -> Vec<u8> {
    let properties = r#"<w:headerReference w:type="default" r:id="rIdHeader"/>
        <w:footerReference w:type="default" r:id="rIdFooter"/>
        <w:pgSz w:w="12240" w:h="15840"/>
        <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="720" w:footer="720"/>"#;
    let boundary = if multiple_sections {
        format!(
            r#"<w:p><w:pPr><w:sectPr>{properties}<w:type w:val="nextPage"/></w:sectPr></w:pPr></w:p>"#
        )
    } else {
        r#"<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>Second page</w:t></w:r></w:p>"#.into()
    };
    let references = if notes {
        r#"<w:r><w:footnoteReference w:id="2"/></w:r><w:r><w:endnoteReference w:id="3"/></w:r>"#
    } else {
        ""
    };
    let document = format!(
        r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
        <w:body>
          <w:p><w:r><w:t>First page</w:t></w:r>{references}</w:p>
          <w:tbl><w:tblPr><w:tblW w:w="4320" w:type="dxa"/></w:tblPr>
            <w:tblGrid><w:gridCol w:w="2160"/><w:gridCol w:w="2160"/></w:tblGrid>
            <w:tr><w:tc><w:tcPr><w:tcW w:w="2160" w:type="dxa"/></w:tcPr>
              <w:p><w:r><w:t>Cell one</w:t></w:r></w:p></w:tc>
              <w:tc><w:tcPr><w:tcW w:w="2160" w:type="dxa"/></w:tcPr>
              <w:p><w:r><w:t>Cell two</w:t></w:r></w:p></w:tc></w:tr>
          </w:tbl>
          {boundary}
          <w:p><w:r><w:t>Final paragraph</w:t></w:r></w:p>
          <w:sectPr>{properties}</w:sectPr>
        </w:body></w:document>"#
    );
    let parts = [
        (
            "[Content_Types].xml",
            r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
            <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
            <Default Extension="xml" ContentType="application/xml"/>
            <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
            <Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/>
            <Override PartName="/word/footer1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"/>
            <Override PartName="/word/footnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml"/>
            <Override PartName="/word/endnotes.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml"/>
          </Types>"#,
        ),
        (
            "_rels/.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rIdDocument" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
          </Relationships>"#,
        ),
        (
            "word/_rels/document.xml.rels",
            r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
            <Relationship Id="rIdHeader" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>
            <Relationship Id="rIdFooter" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer" Target="footer1.xml"/>
            <Relationship Id="rIdFootnotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/footnotes" Target="footnotes.xml"/>
            <Relationship Id="rIdEndnotes" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/endnotes" Target="endnotes.xml"/>
          </Relationships>"#,
        ),
        ("word/document.xml", document.as_str()),
        (
            "word/header1.xml",
            r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:p><w:r><w:t xml:space="preserve">Header </w:t></w:r>
              <w:fldSimple w:instr=" PAGE "><w:r><w:t>99</w:t></w:r></w:fldSimple>
              <w:r><w:t xml:space="preserve"> / </w:t></w:r>
              <w:fldSimple w:instr=" NUMPAGES "><w:r><w:t>99</w:t></w:r></w:fldSimple>
            </w:p></w:hdr>"#,
        ),
        (
            "word/footer1.xml",
            r#"<w:ftr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:p><w:r><w:t xml:space="preserve">Footer </w:t></w:r>
              <w:fldSimple w:instr=" PAGE "><w:r><w:t>99</w:t></w:r></w:fldSimple>
            </w:p></w:ftr>"#,
        ),
        (
            "word/footnotes.xml",
            r#"<w:footnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:footnote w:id="2"><w:p><w:r><w:t>Footnote fixture</w:t></w:r></w:p></w:footnote>
          </w:footnotes>"#,
        ),
        (
            "word/endnotes.xml",
            r#"<w:endnotes xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
            <w:endnote w:id="3"><w:p><w:r><w:t>Endnote fixture</w:t></w:r></w:p></w:endnote>
          </w:endnotes>"#,
        ),
    ];
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, contents) in parts {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(contents.as_bytes()).unwrap();
    }
    archive.finish().unwrap().into_inner()
}

/// Reference implementation of the old, two-build opening sequence.
fn legacy_display_list(bytes: &[u8]) -> DisplayList {
    let parsed =
        parse_docx_s9_wire_with_limits(bytes, S9ParseOptions::default(), &ParseLimits::default())
            .unwrap();
    let engine = EngineSession::new(0x4d54_444f_4358);
    seed_from_docx(engine.doc(), bytes).unwrap();
    let probe = region_request(&parsed.document.package, None).unwrap();
    let requirements: Value = serde_json::from_str(
        &engine
            .layout_font_requirements_json(&probe.to_string())
            .unwrap(),
    )
    .unwrap();
    let fonts = RegisteredFonts::load(&requirements).unwrap();
    let request = region_request(&parsed.document.package, Some(&fonts.layout_chain_ids)).unwrap();
    engine
        .layout_document_with_regions_retained_json(&request.to_string())
        .unwrap();
    engine
        .build_display_list_frame(
            &json!({ "fontChains": fonts.layout_chain_ids }).to_string(),
            0,
        )
        .unwrap();
    engine.apply_and_layout("body", 0).unwrap();
    assert_eq!(engine.stats().display_builds, 2);
    engine.with_display_list(Clone::clone).unwrap()
}

fn text(primitives: &[Primitive]) -> String {
    primitives
        .iter()
        .filter_map(|primitive| match primitive {
            Primitive::Text(run) => Some(run.text.as_str()),
            Primitive::GlyphRun(run) => Some(run.text.as_str()),
            _ => None,
        })
        .collect()
}

fn compare_with_legacy(multiple_sections: bool, notes: bool) -> DisplayList {
    let bytes = region_docx(multiple_sections, notes);
    let legacy = legacy_display_list(&bytes);
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), &bytes).unwrap();
    let mut renderer = DocxRenderer::open(source.path(), OpenTrace::default()).unwrap();
    let optimized = renderer.search_display_list();
    assert_eq!(
        *optimized, legacy,
        "opening once must preserve every page and primitive"
    );
    for page_index in 0..optimized.pages.len() {
        let page = renderer
            .render_page(page_index, OpenTrace::default())
            .unwrap();
        assert!(page
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| { pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128 }));
    }
    assert_eq!(std::fs::read(source.path()).unwrap(), bytes);
    (*optimized).clone()
}

#[test]
fn single_pass_preserves_headers_footers_and_page_fields() {
    let list = compare_with_legacy(false, false);
    assert_eq!(list.pages.len(), 2);
    for (index, page) in list.pages.iter().enumerate() {
        assert_eq!(
            text(&page.header.as_ref().unwrap().primitives),
            format!("Header {} / 2", index + 1)
        );
        assert_eq!(
            text(&page.footer.as_ref().unwrap().primitives),
            format!("Footer {}", index + 1)
        );
    }
    assert!(text(&list.pages[0].primitives).contains("Cell one"));
    assert!(text(&list.pages[0].primitives).contains("Cell two"));
}

#[test]
fn single_pass_preserves_multiple_sections() {
    let list = compare_with_legacy(true, false);
    assert_eq!(list.pages.len(), 2);
    assert_ne!(list.pages[0].section_id, list.pages[1].section_id);
    assert!(list
        .pages
        .iter()
        .all(|page| page.header.is_some() && page.footer.is_some()));
}

#[test]
fn single_pass_preserves_footnotes_and_endnotes() {
    let list = compare_with_legacy(false, true);
    let note_text: String = list
        .pages
        .iter()
        .flat_map(|page| &page.note_areas)
        .map(|area| text(&area.primitives))
        .collect();
    assert!(note_text.contains("Footnote fixture"), "{note_text}");
    assert!(note_text.contains("Endnote fixture"), "{note_text}");
}

#[test]
fn display_extras_keeps_only_fonts_and_optional_measured_regions() {
    let chains = BTreeMap::from([("calibri|0|0".into(), vec![0])]);
    let reply = json!({
        "layout": { "pages": [{ "pageIndex": 0 }] },
        "headersFooters": { "variants": [{ "rId": "rIdHeader" }] },
        "notesConverged": true
    })
    .to_string();
    let extras: Value = serde_json::from_str(&display_extras(&reply, &chains).unwrap()).unwrap();
    assert_eq!(extras["fontChains"]["calibri|0|0"], json!([0]));
    assert_eq!(extras["headersFooters"]["variants"][0]["rId"], "rIdHeader");
    assert!(extras.get("layout").is_none());
    for reply in [r#"{"layout":{}}"#, r#"{"headersFooters":null}"#] {
        let extras: Value = serde_json::from_str(&display_extras(reply, &chains).unwrap()).unwrap();
        assert!(extras.get("headersFooters").is_none());
    }
    assert!(display_extras("invalid", &chains).is_err());
}
