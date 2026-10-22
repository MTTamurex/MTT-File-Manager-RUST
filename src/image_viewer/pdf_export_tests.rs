use super::encode_frame_to_pdf;
use image::{Rgba, RgbaImage};
use lopdf::{content::Content, Document, Object};
use std::io::{self, Write};

fn export(image: RgbaImage) -> Document {
    let mut bytes = Vec::new();
    encode_frame_to_pdf(image, &mut bytes).expect("image should export");
    Document::load_mem(&bytes).expect("exported PDF should parse")
}

fn image_stream(doc: &Document) -> &lopdf::Stream {
    let images: Vec<_> = doc
        .objects
        .values()
        .filter_map(|object| object.as_stream().ok())
        .filter(|stream| {
            stream.dict.get(b"Subtype").and_then(Object::as_name).ok() == Some(b"Image")
        })
        .collect();
    assert_eq!(images.len(), 1);
    images[0]
}

#[test]
fn preserves_rgb_pixels_and_composites_alpha_over_white() {
    let pixels = vec![255, 0, 0, 255, 0, 0, 255, 128, 0, 255, 0, 0];
    let doc = export(RgbaImage::from_raw(3, 1, pixels).unwrap());
    let stream = image_stream(&doc);
    assert_eq!(stream.dict.get(b"Width").unwrap().as_i64().unwrap(), 3);
    assert_eq!(stream.dict.get(b"Height").unwrap().as_i64().unwrap(), 1);
    assert_eq!(
        stream
            .dict
            .get(b"BitsPerComponent")
            .unwrap()
            .as_i64()
            .unwrap(),
        8
    );
    assert_eq!(
        stream.dict.get(b"ColorSpace").unwrap().as_name().unwrap(),
        b"DeviceRGB"
    );
    assert_eq!(
        stream.dict.get(b"Filter").unwrap().as_name().unwrap(),
        b"FlateDecode"
    );
    assert!(stream.dict.get(b"Interpolate").unwrap().as_bool().unwrap());
    assert!(!stream.dict.has(b"SMask"));
    assert_eq!(
        stream.decompressed_content().unwrap(),
        [255, 0, 0, 127, 127, 255, 255, 255, 255]
    );
}

#[test]
fn preserves_page_dimensions_and_300_dpi_placement() {
    for (width, height) in [(300, 600), (600, 300), (1, 1)] {
        let doc = export(RgbaImage::from_pixel(
            width,
            height,
            Rgba([12, 34, 56, 255]),
        ));
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1);
        let page_id = pages[&1];
        let page = doc.get_dictionary(page_id).unwrap();
        let bounds = page.get(b"MediaBox").unwrap().as_array().unwrap();
        let expected_w = width as f32 * 72.0 / 300.0;
        let expected_h = height as f32 * 72.0 / 300.0;
        assert!((bounds[2].as_float().unwrap() - expected_w).abs() < 0.001);
        assert!((bounds[3].as_float().unwrap() - expected_h).abs() < 0.001);

        let content = Content::decode(&doc.get_page_content(page_id)).unwrap();
        let matrices: Vec<_> = content
            .operations
            .iter()
            .filter(|op| op.operator == "cm")
            .collect();
        assert_eq!(matrices.len(), 1);
        let matrix: Vec<_> = matrices[0]
            .operands
            .iter()
            .map(|v| v.as_float().unwrap())
            .collect();
        let expected = [expected_w, 0.0, 0.0, expected_h, 0.0, 0.0];
        for (actual, expected) in matrix.iter().zip(expected) {
            assert!((actual - expected).abs() < 0.001);
        }
        assert_eq!(
            content
                .operations
                .iter()
                .filter(|op| op.operator == "Do")
                .count(),
            1
        );
    }
}

#[test]
fn does_not_resize_large_images_or_convert_near_gray_pixels() {
    // Exceeds printpdf's default 2 MB image optimization threshold.
    let (width, height) = (1024, 768);
    let expected = [120, 121, 122];
    let doc = export(RgbaImage::from_pixel(
        width,
        height,
        Rgba([120, 121, 122, 255]),
    ));
    let stream = image_stream(&doc);
    assert_eq!(
        stream.dict.get(b"Width").unwrap().as_i64().unwrap(),
        i64::from(width)
    );
    assert_eq!(
        stream.dict.get(b"Height").unwrap().as_i64().unwrap(),
        i64::from(height)
    );
    let pixels = stream.decompressed_content().unwrap();
    assert_eq!(pixels.len(), width as usize * height as usize * 3);
    assert!(pixels.chunks_exact(3).all(|pixel| pixel == expected));
}

#[test]
fn propagates_writer_errors() {
    struct FailingWriter;
    impl Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::StorageFull,
                "test writer is full",
            ))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let error = encode_frame_to_pdf(RgbaImage::new(1, 1), &mut FailingWriter).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::StorageFull);
}

#[test]
fn rejects_zero_dimensions_before_writing() {
    for (width, height) in [(0, 0), (0, 1), (1, 0)] {
        let mut bytes = Vec::new();
        let error = encode_frame_to_pdf(RgbaImage::new(width, height), &mut bytes).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(bytes.is_empty());
    }
}

fn nested_pdf(open: &str, close: &str, depth: usize) -> Vec<u8> {
    let nested = format!("{} null {}", open.repeat(depth), close.repeat(depth));
    let mut bytes = b"%PDF-1.4\n".to_vec();
    let root_offset = bytes.len();
    write!(
        bytes,
        "1 0 obj\n<< /Type /Catalog /Pages 2 0 R /X {nested} >>\nendobj\n"
    )
    .unwrap();
    let pages_offset = bytes.len();
    bytes.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [] /Count 0 >>\nendobj\n");
    let xref_offset = bytes.len();
    write!(
        bytes,
        "xref\n0 3\n0000000000 65535 f \n{root_offset:010} 00000 n \n{pages_offset:010} 00000 n \ntrailer\n<< /Size 3 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n"
    ).unwrap();
    bytes
}

#[test]
fn deeply_nested_pdf_objects_do_not_abort_the_process() {
    const CHILD_FLAG: &str = "MTT_TEST_PDF_NESTING_CHILD";
    if std::env::var_os(CHILD_FLAG).is_none() {
        // A regression here aborts instead of unwinding: isolate the parser so
        // the test runner can report the failure without losing the whole suite.
        let module = module_path!().split_once("::").unwrap().1;
        let test_name = format!("{module}::deeply_nested_pdf_objects_do_not_abort_the_process");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &test_name, "--nocapture"])
            .env(CHILD_FLAG, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("nested PDF parser child timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "nested PDF parser child failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }

    for (open, close) in [("[", "]"), ("<< /X ", ">>")] {
        let shallow = Document::load_mem(&nested_pdf(open, close, 4)).unwrap();
        assert!(shallow.catalog().unwrap().has(b"X"));
        let bytes = nested_pdf(open, close, 10_380);
        // lopdf may reject the document outright or skip the malformed catalog.
        if let Ok(doc) = Document::load_mem(&bytes) {
            assert!(
                doc.catalog().is_err(),
                "overly nested catalog must not be accepted"
            );
        }
    }
}

#[test]
#[ignore = "requires the bundled Pdfium runtime"]
fn exported_pdf_renders_with_pdfium() {
    use pdfium_render::prelude::*;

    let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("vendor")
        .join("pdfium.dll");
    let pdfium = Pdfium::new(Pdfium::bind_to_library(library).expect("bundled Pdfium should load"));
    for (width, height) in [(600, 300), (300, 600)] {
        let image = RgbaImage::from_fn(width, height, |x, y| {
            match (x < width / 2, y < height / 2) {
                (true, true) => Rgba([255, 0, 0, 255]),
                (false, true) => Rgba([0, 255, 0, 255]),
                (true, false) => Rgba([0, 0, 255, 128]),
                (false, false) => Rgba([0, 0, 0, 0]),
            }
        });
        let mut bytes = Vec::new();
        encode_frame_to_pdf(image, &mut bytes).unwrap();
        let doc = pdfium.load_pdf_from_byte_slice(&bytes, None).unwrap();
        assert_eq!(doc.pages().len(), 1);
        let page = doc.pages().get(0).unwrap();
        assert!((page.width().value - width as f32 * 72.0 / 300.0).abs() < 0.001);
        assert!((page.height().value - height as f32 * 72.0 / 300.0).abs() < 0.001);
        let bitmap = page
            .render_with_config(
                &PdfRenderConfig::new().set_target_size(width as i32, height as i32),
            )
            .unwrap();
        assert_eq!(bitmap.width(), width as i32);
        assert_eq!(bitmap.height(), height as i32);
        let pixels = bitmap.as_rgba_bytes();
        for (x, y, expected) in [
            (width / 4, height / 4, [255, 0, 0, 255]),
            (3 * width / 4, height / 4, [0, 255, 0, 255]),
            (width / 4, 3 * height / 4, [127, 127, 255, 255]),
            (3 * width / 4, 3 * height / 4, [255, 255, 255, 255]),
        ] {
            let offset = ((y * width + x) * 4) as usize;
            assert_eq!(&pixels[offset..offset + 4], &expected);
        }
    }
}
