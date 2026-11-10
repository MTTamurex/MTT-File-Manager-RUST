/// Cached artwork read without consulting the source filesystem.
/// The fingerprint must be checked asynchronously before treating it as fresh.
pub(crate) struct FolderPreviewSnapshot {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub created_at: i64,
    pub fingerprint: Option<String>,
}

impl FolderPreviewSnapshot {
    pub fn into_pixels(self) -> (Vec<u8>, u32, u32, i64) {
        (self.pixels, self.width, self.height, self.created_at)
    }
}
