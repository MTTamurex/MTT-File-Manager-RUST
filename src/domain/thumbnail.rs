use std::path::PathBuf;

/// Dados de thumbnail extraídos de arquivo
#[derive(Clone)]
pub struct ThumbnailData {
    pub path: PathBuf,
    pub image_data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub generation: usize, // Rastreia a validade da extração
    pub not_found: bool,   // Arquivo não existe mais no disco
}
