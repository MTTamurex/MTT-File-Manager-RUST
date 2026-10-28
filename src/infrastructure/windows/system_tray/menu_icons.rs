use super::{
    graphics::{rasterize, Bitmap},
    model::LocationKind,
};
use windows::Win32::Graphics::Gdi::HBITMAP;

/// Native menu bitmaps live until the whole popup hierarchy has been destroyed.
pub(super) struct MenuIcons([Option<Bitmap>; 4]);

impl MenuIcons {
    pub(super) fn new(size: u32) -> Self {
        Self(std::array::from_fn(|index| {
            let (svg, color) = match LocationKind::ALL[index] {
                LocationKind::Folder => (
                    crate::embedded_assets::get_icon("folder_filled")?,
                    "#388fe5",
                ),
                LocationKind::Tag => (crate::embedded_assets::get_icon("tag")?, "#9764d2"),
                LocationKind::Computer => (COMPUTER.as_bytes(), "#388fe5"),
                LocationKind::RecycleBin => {
                    (crate::embedded_assets::get_icon("delete")?, "#6a8a9e")
                }
            };
            let svg = std::str::from_utf8(svg)
                .ok()?
                .replace("currentColor", color);
            Bitmap::from_pixmap(&rasterize(&svg, size)?)
        }))
    }

    pub(super) fn get(&self, kind: LocationKind) -> Option<HBITMAP> {
        self.0[kind as usize].as_ref().map(|bitmap| bitmap.0)
    }
}

const COMPUTER: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='currentColor' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><rect x='2' y='3' width='20' height='14' rx='2'/><path d='M8 21H16M12 17V21'/></svg>";
