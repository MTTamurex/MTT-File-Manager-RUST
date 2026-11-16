mod display_profile;
mod regions;
mod small_caps;

use std::io::{Cursor, Write};

use super::*;

fn synthetic_docx() -> Vec<u8> {
    synthetic_docx_with_text_run("<w:r><w:t>DOCX viewer fixture</w:t></w:r>")
}

fn synthetic_docx_with_text_run(text_run: &str) -> Vec<u8> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut archive = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        for (name, contents) in [
            (
                "[Content_Types].xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#,
            ),
            (
                "word/_rels/document.xml.rels",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rIdImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/image1.png"/>
</Relationships>"#,
            ),
            (
                "word/document.xml",
                r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture">
  <w:body>
    <w:p w14:paraId="11111111"><w:r><w:t>DOCX viewer fixture</w:t></w:r></w:p>
    <w:p w14:paraId="22222222"><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="914400" cy="914400"/><wp:docPr id="1" name="Fixture picture"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="0" name="Fixture image"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImage"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="914400"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>
    <w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr>
  </w:body>
</w:document>"#,
            ),
        ] {
            archive.start_file(name, options).unwrap();
            if name == "word/document.xml" {
                let contents =
                    contents.replace("<w:r><w:t>DOCX viewer fixture</w:t></w:r>", text_run);
                archive.write_all(contents.as_bytes()).unwrap();
            } else {
                archive.write_all(contents.as_bytes()).unwrap();
            }
        }
        let mut image = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([240, 20, 20, 255]),
        ))
        .write_to(&mut image, image::ImageFormat::Png)
        .unwrap();
        archive
            .start_file("word/media/image1.png", options)
            .unwrap();
        archive.write_all(&image.into_inner()).unwrap();
        archive.finish().unwrap();
    }
    cursor.into_inner()
}

#[test]
fn opens_and_rasterizes_a_synthetic_docx_page() {
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), synthetic_docx()).unwrap();

    let mut renderer = DocxRenderer::open(source.path(), OpenTrace::default()).unwrap();
    assert_eq!(renderer.page_sizes().len(), 1);
    let page = renderer.render_page(0, OpenTrace::default()).unwrap();

    let (layout_width, layout_height) = renderer.page_sizes()[0];
    assert_eq!(page.width, (layout_width * 2.0).ceil() as usize);
    assert_eq!(page.height, (layout_height * 2.0).ceil() as usize);
    assert!(page.width > 0);
    assert!(page.height > 0);
    assert_eq!(page.rgba.len(), page.width * page.height * 4);
    assert_eq!(page.skipped_images, 0);
    assert!(page
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[0] < 128 && pixel[1] < 128 && pixel[2] < 128));
    assert!(page
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .any(|pixel| pixel[0] > 200 && pixel[1] < 80 && pixel[2] < 80));
}
