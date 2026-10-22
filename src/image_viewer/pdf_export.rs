//! Lossless, single-page image export without PDF input parsing.

use std::io::{self, Write};

use printpdf::{
    Layer, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, RawImage, RawImageData, RawImageFormat,
    XObject, XObjectId, XObjectTransform,
};

/// Encode RGBA pixels at 300 DPI, compositing alpha over white.
pub(super) fn encode_frame_to_pdf(
    rgba: image::RgbaImage,
    writer: &mut impl Write,
) -> io::Result<()> {
    let (width, height) = rgba.dimensions();
    if width == 0 || height == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "image dimensions must be nonzero",
        ));
    }
    let rgb_bytes = rgba
        .pixels()
        .flat_map(|p| {
            let a = p[3] as u32;
            let inv = 255 - a;
            [
                ((p[0] as u32 * a + 255 * inv + 128) / 255) as u8,
                ((p[1] as u32 * a + 255 * inv + 128) / 255) as u8,
                ((p[2] as u32 * a + 255 * inv + 128) / 255) as u8,
            ]
        })
        .collect();
    // Release the original pixels before the PDF serializer copies the RGB data.
    drop(rgba);

    let mut doc = PdfDocument::new("Image export");
    let image_id = XObjectId::new();
    let image = RawImage {
        width: width as usize,
        height: height as usize,
        data_format: RawImageFormat::RGB8,
        pixels: RawImageData::U8(rgb_bytes),
        tag: Vec::new(),
    };
    // add_image() clones its input; move the pixels into the resource instead.
    doc.resources
        .xobjects
        .map
        .insert(image_id.clone(), XObject::Image(image));
    let layer_id = doc.add_layer(&Layer::new("Layer 1"));

    // Keep the previous pixel-to-mm conversion and image placement.
    const PX_TO_MM: f64 = 72.0 / 300.0 * 0.352_777_8;
    doc.with_pages(vec![PdfPage::new(
        Mm((width as f64 * PX_TO_MM) as f32),
        Mm((height as f64 * PX_TO_MM) as f32),
        vec![
            Op::BeginLayer { layer_id },
            Op::UseXobject {
                id: image_id,
                transform: XObjectTransform {
                    dpi: Some(300.0),
                    ..Default::default()
                },
            },
            Op::EndLayer,
        ],
    )]);

    // No automatic resizing, grayscale conversion, or lossy image compression.
    // With printpdf's image codecs disabled, RGB pixels use lossless FlateDecode.
    let options = PdfSaveOptions {
        image_optimization: None,
        ..Default::default()
    };
    let mut pdf = doc.to_lopdf_document(&options, &mut Vec::new());
    drop(doc);
    // The new API doesn't expose interpolation on RawImage. Preserve the old
    // image XObject setting rather than accepting the serializer's new default.
    for object in pdf.objects.values_mut() {
        if let Ok(stream) = object.as_stream_mut() {
            if stream.dict.get(b"Subtype").and_then(|v| v.as_name()).ok() == Some(b"Image") {
                stream.dict.set("Interpolate", true);
            }
        }
    }
    pdf.compress();
    // printpdf::save_writer discards I/O errors; save_to preserves them and
    // avoids allocating a second buffer for the entire serialized document.
    pdf.save_to(writer)
}

#[cfg(test)]
#[path = "pdf_export_tests.rs"]
mod tests;
