//! Windows DPAPI: encrypts data so only the current Windows user on this PC can decrypt it.

use windows::Win32::Foundation::{LocalFree, HLOCAL};
use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

fn run(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB::default();
    unsafe {
        let ok = if protect {
            CryptProtectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        ok.ok()?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as _));
        Some(bytes)
    }
}

pub fn protect(data: &[u8]) -> Option<Vec<u8>> { run(data, true) }
pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> { run(data, false) }

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let blob = super::protect(b"secret").unwrap();
        assert_ne!(&blob[..], b"secret");
        assert_eq!(super::unprotect(&blob).unwrap(), b"secret");
    }
}
