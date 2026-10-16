use crate::domain::file_entry::IconSize;
use windows::Win32::UI::Shell::{
    SHGetStockIconInfo, SHGSI_ICONLOCATION, SHSTOCKICONINFO, SIID_SERVERSHARE,
};

/// Extract the native shared-network-folder icon from Windows' local resources.
pub fn extract_network_location_icon(
    size: IconSize,
) -> Result<(Vec<u8>, u32, u32), Box<dyn std::error::Error>> {
    let mut info = SHSTOCKICONINFO {
        cbSize: std::mem::size_of::<SHSTOCKICONINFO>() as u32,
        ..Default::default()
    };
    // Request only the resource location: this does not allocate an HICON or
    // query a network path. The resource extractor handles icon ownership.
    unsafe { SHGetStockIconInfo(SIID_SERVERSHARE, SHGSI_ICONLOCATION, &mut info)? };
    let path_units = info.szPath;
    let end = path_units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(path_units.len());
    let path = String::from_utf16(&path_units[..end])?;
    if path.is_empty() {
        return Err("Network folder icon resource is unavailable".into());
    }
    let index = info.iIcon;
    super::file_icons::extract_icon_resource(&format!("{path},{index}"), size)
}
