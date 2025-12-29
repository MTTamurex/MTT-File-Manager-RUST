use std::path::PathBuf;

/// Dados de thumbnail extraídos de arquivo
pub struct ThumbnailData {
    pub path: PathBuf,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: usize, // Rastreia a validade da extração
}
