//! SEC: Authenticated integrity for the binary index files.
//!
//! Replaces the legacy CRC32 footer with a 32-byte HMAC-SHA256 computed over
//! the entire file payload (header + arena + records + hardlinks + reparse).
//! The HMAC key is a random 32-byte secret generated once and persisted —
//! sealed at rest with DPAPI in `LocalMachine` scope so:
//!
//!   * Only code running on the same physical machine can unseal it
//!     (DPAPI's `CRYPTPROTECT_LOCAL_MACHINE` derivation).
//!   * The key file itself sits inside `data_dir()`, which CRIT-1 already
//!     locks down to `SYSTEM` + `BUILTIN\Administrators` + read-only `Users`.
//!
//! Threat addressed: an attacker who somehow obtained write access to the
//! `index_X.bin` file (e.g. after a future ACL regression, or from a backup
//! restored to another machine) would otherwise be able to forge a CRC32 and
//! poison the in-memory index of a LocalSystem service. With HMAC-SHA256, a
//! forgery requires the per-machine key — which itself never leaves DPAPI
//! plaintext on disk and cannot be decrypted on a different machine.
//!
//! On HMAC mismatch the file is treated as corrupt/tampered and discarded.
//! On legacy magic (`MTTIDX01`) it is treated as missing and rebuilt.
use std::path::PathBuf;

use windows::core::PCWSTR;
use windows::Win32::Foundation::LocalFree;
use windows::Win32::Security::Cryptography::{
    BCryptCloseAlgorithmProvider, BCryptCreateHash, BCryptDestroyHash, BCryptFinishHash,
    BCryptGenRandom, BCryptHashData, BCryptOpenAlgorithmProvider, CryptProtectData,
    CryptUnprotectData, BCRYPT_ALG_HANDLE, BCRYPT_ALG_HANDLE_HMAC_FLAG, BCRYPT_HASH_HANDLE,
    BCRYPT_SHA256_ALGORITHM, BCRYPT_USE_SYSTEM_PREFERRED_RNG, CRYPTPROTECT_LOCAL_MACHINE,
    CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};

const HMAC_KEY_SIZE: usize = 32;
const KEY_FILE_NAME: &str = "hmac_key.bin";

pub const HMAC_OUTPUT_SIZE: usize = 32;

/// Returns the absolute path of the DPAPI-sealed HMAC key file.
fn key_file_path() -> PathBuf {
    super::data_dir().join(KEY_FILE_NAME)
}

/// Returns the per-machine HMAC key, generating + sealing it on first call.
/// Errors are propagated so the caller can decide whether to refuse to
/// load/save (we always do — without the key we cannot authenticate the file).
pub fn machine_key() -> Result<Vec<u8>, String> {
    let path = key_file_path();
    if path.exists() {
        let blob = std::fs::read(&path).map_err(|e| format!("read HMAC key file: {}", e))?;
        let key = dpapi_unprotect(&blob)?;
        if key.len() != HMAC_KEY_SIZE {
            return Err(format!("HMAC key blob has unexpected length {}", key.len()));
        }
        return Ok(key);
    }

    // First-run: generate, seal, persist atomically.
    let mut key = vec![0u8; HMAC_KEY_SIZE];
    bcrypt_random(&mut key)?;
    let sealed = dpapi_protect(&key)?;
    let tmp = path.with_extension("bin.tmp");
    std::fs::write(&tmp, &sealed).map_err(|e| format!("write HMAC key tmp: {}", e))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename HMAC key: {}", e))?;
    Ok(key)
}

pub struct HmacSha256 {
    _alg: AlgGuard,
    hash: HashGuard,
}

impl HmacSha256 {
    pub fn new(key: &[u8]) -> Result<Self, String> {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        let mut hash = BCRYPT_HASH_HANDLE::default();

        unsafe {
            let status = BCryptOpenAlgorithmProvider(
                &mut alg,
                BCRYPT_SHA256_ALGORITHM,
                PCWSTR::null(),
                BCRYPT_ALG_HANDLE_HMAC_FLAG,
            );
            if status.is_err() {
                return Err(format!("BCryptOpenAlgorithmProvider: 0x{:08x}", status.0));
            }

            let status = BCryptCreateHash(alg, &mut hash, None, Some(key), 0);
            if status.is_err() {
                let _ = BCryptCloseAlgorithmProvider(alg, 0);
                return Err(format!("BCryptCreateHash: 0x{:08x}", status.0));
            }
        }

        Ok(Self {
            _alg: AlgGuard(alg),
            hash: HashGuard(hash),
        })
    }

    pub fn update(&mut self, data: &[u8]) -> Result<(), String> {
        unsafe {
            let status = BCryptHashData(self.hash.0, data, 0);
            if status.is_err() {
                return Err(format!("BCryptHashData: 0x{:08x}", status.0));
            }
        }
        Ok(())
    }

    pub fn finalize(self) -> Result<[u8; HMAC_OUTPUT_SIZE], String> {
        let mut tag = [0u8; HMAC_OUTPUT_SIZE];

        unsafe {
            let status = BCryptFinishHash(self.hash.0, &mut tag, 0);
            if status.is_err() {
                return Err(format!("BCryptFinishHash: 0x{:08x}", status.0));
            }
        }

        Ok(tag)
    }
}

/// Constant-time equality check for HMAC tags, to avoid leaking the prefix
/// length of a forged tag through timing differences.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

fn bcrypt_random(buf: &mut [u8]) -> Result<(), String> {
    unsafe {
        let status = BCryptGenRandom(None, buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG);
        if status.is_err() {
            return Err(format!("BCryptGenRandom: 0x{:08x}", status.0));
        }
    }
    Ok(())
}

fn dpapi_protect(plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &mut input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptProtectData: {}", e))?;
    }

    let sealed =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData as *mut _,
        )));
    }
    Ok(sealed)
}

fn dpapi_unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
    let mut input = CRYPT_INTEGER_BLOB {
        cbData: sealed.len() as u32,
        pbData: sealed.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &mut input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_LOCAL_MACHINE | CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
        .map_err(|e| format!("CryptUnprotectData: {}", e))?;
    }

    let plain =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        let _ = LocalFree(Some(windows::Win32::Foundation::HLOCAL(
            output.pbData as *mut _,
        )));
    }
    Ok(plain)
}

struct AlgGuard(BCRYPT_ALG_HANDLE);
impl Drop for AlgGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptCloseAlgorithmProvider(self.0, 0);
        }
    }
}

struct HashGuard(BCRYPT_HASH_HANDLE);
impl Drop for HashGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = BCryptDestroyHash(self.0);
        }
    }
}
