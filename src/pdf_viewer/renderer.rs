//! PDF page rendering using the Windows.Data.Pdf API.
//!
//! Uses the built-in Windows 10+ PDF renderer — no external DLLs required.
//! Pages are rendered to PNG streams and decoded to RGBA pixel buffers.

use std::path::Path;
use windows::core::HSTRING;
use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
use windows::Storage::StorageFile;
use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};

/// A loaded PDF document ready for page rendering.
pub struct PdfRenderer {
    document: PdfDocument,
    page_count: u32,
}

/// A rendered PDF page as raw RGBA pixels.
pub struct RenderedPage {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

impl PdfRenderer {
    /// Open a PDF file from disk.
    pub fn open(path: &Path) -> Result<Self, String> {
        let path_hstring = HSTRING::from(path.as_os_str());

        let file = StorageFile::GetFileFromPathAsync(&path_hstring)
            .map_err(|e| format!("GetFileFromPathAsync: {e}"))?
            .get()
            .map_err(|e| format!("GetFile: {e}"))?;

        let document = PdfDocument::LoadFromFileAsync(&file)
            .map_err(|e| format!("LoadFromFileAsync: {e}"))?
            .get()
            .map_err(|e| format!("LoadPdf: {e}"))?;

        let page_count = document.PageCount().map_err(|e| e.to_string())?;

        Ok(Self {
            document,
            page_count,
        })
    }

    /// Total number of pages in the document.
    #[inline]
    pub fn page_count(&self) -> u32 {
        self.page_count
    }

    /// Natural size (width, height) of a page in device-independent pixels.
    pub fn page_size(&self, index: u32) -> Result<(f32, f32), String> {
        let page = self.document.GetPage(index).map_err(|e| e.to_string())?;
        let size = page.Size().map_err(|e| e.to_string())?;
        Ok((size.Width, size.Height))
    }

    /// Render a single page to RGBA pixels at the requested pixel dimensions.
    ///
    /// The Windows PDF API renders the page to a PNG stream, which is then
    /// decoded to an RGBA buffer via the `image` crate.
    pub fn render_page(
        &self,
        index: u32,
        target_width: u32,
        target_height: u32,
    ) -> Result<RenderedPage, String> {
        let page = self.document.GetPage(index).map_err(|e| e.to_string())?;

        let stream =
            InMemoryRandomAccessStream::new().map_err(|e| format!("NewStream: {e}"))?;

        let options =
            PdfPageRenderOptions::new().map_err(|e| format!("NewRenderOptions: {e}"))?;
        options
            .SetDestinationWidth(target_width)
            .map_err(|e| e.to_string())?;
        options
            .SetDestinationHeight(target_height)
            .map_err(|e| e.to_string())?;

        page.RenderWithOptionsToStreamAsync(&stream, &options)
            .map_err(|e| format!("RenderToStream: {e}"))?
            .get()
            .map_err(|e| format!("RenderToStream.get: {e}"))?;

        // Read PNG bytes from the in-memory stream
        let byte_count = stream.Size().map_err(|e| e.to_string())? as u32;
        let input = stream
            .GetInputStreamAt(0)
            .map_err(|e| e.to_string())?;
        let reader = DataReader::CreateDataReader(&input).map_err(|e| e.to_string())?;

        let loaded = reader
            .LoadAsync(byte_count)
            .map_err(|e| e.to_string())?
            .get()
            .map_err(|e| e.to_string())?;

        let mut png_data = vec![0u8; loaded as usize];
        reader.ReadBytes(&mut png_data).map_err(|e| e.to_string())?;

        // Decode PNG → RGBA
        let img = image::load_from_memory(&png_data)
            .map_err(|e| format!("PNG decode: {e}"))?
            .into_rgba8();

        Ok(RenderedPage {
            width: img.width(),
            height: img.height(),
            pixels: img.into_raw(),
        })
    }
}
