/// Client-side named-pipe access: FILE_GENERIC_READ | FILE_WRITE_DATA.
///
/// Keep read attributes, read control and synchronization for existing client
/// operations, but do not request FILE_GENERIC_WRITE: its FILE_APPEND_DATA bit
/// aliases FILE_CREATE_PIPE_INSTANCE and prevents a future restrictive DACL from
/// accepting ordinary clients. This is a requested-access mask, not a server ACL.
/// The numeric value keeps the protocol crate independent of the Windows crate.
pub const PIPE_CLIENT_ACCESS: u32 = 0x0012_008B;
