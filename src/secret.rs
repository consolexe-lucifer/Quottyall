//! Local, encrypted storage for the subscription / API keys entered by the
//! user (MiniMax M Plan, DeepSeek).
//!
//! These are bearer credentials for the user's own accounts: they must never
//! end up in `settings.json` (plain JSON, easy to sync or screenshot), in a log
//! line, or in this repository. So each key lives in its own file, sealed with
//! Windows DPAPI under the *current user* — the same mechanism the Windows
//! credential manager, Chrome and Edge use. DPAPI ties the ciphertext to the
//! login credentials of that user, so copying the file to another machine or
//! another account yields nothing.
//!
//! Layout: `%APPDATA%\Quotty\<slot>.key`, a five-byte header plus the DPAPI
//! blob. On non-Windows builds there is no DPAPI, and a plaintext fallback
//! would be exactly the thing we promised not to do, so the store reports
//! "unsupported" and the provider stays unconfigured instead.

use std::path::PathBuf;

/// File header so a corrupt or foreign file is rejected before it reaches
/// DPAPI. Bumped only if the on-disk layout changes.
const MAGIC: &[u8; 5] = b"QTMM1";

/// Refuse absurd input instead of handing it to the API: a real key is a few
/// dozen characters, and anything huge is a paste accident.
const MAX_KEY_BYTES: usize = 4096;

/// A named secret. The name is part of the file name, so it is restricted to
/// characters that cannot climb out of the settings directory.
#[derive(Clone, Copy, Debug)]
pub struct Slot(&'static str);

impl Slot {
    /// MiniMax M Plan API key ("Subscription Key" in the MiniMax console).
    pub const MINIMAX: Slot = Slot("minimax");
    /// DeepSeek open-platform API key.
    pub const DEEPSEEK: Slot = Slot("deepseek");

    fn file(self) -> Option<PathBuf> {
        let name = self.0;
        // Defence in depth: the constants above are safe, but the rule belongs
        // next to the place that builds a path from the value.
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return None;
        }
        crate::config::Settings::data_dir().map(|d| d.join(format!("{name}.key")))
    }
}

/// Whether secure storage is available on this build.
pub fn supported() -> bool {
    cfg!(windows)
}

/// Whether a key has been stored for `slot`. Cheap (a metadata call), used by
/// the UI and by the provider to tell "not configured" from "fetch failed".
pub fn exists(slot: Slot) -> bool {
    if !supported() {
        return false;
    }
    slot.file().is_some_and(|p| std::fs::metadata(p).is_ok())
}

/// Last four characters of the stored key, for "ключ сохранён ····1234".
/// Never returns more than that: the UI must not be a way to read the key back.
pub fn hint(slot: Slot) -> Option<String> {
    let key = load(slot).ok()?;
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    Some(tail)
}

/// Validate and store `key` encrypted under the current Windows user.
/// Overwrites any previous key atomically (write to a temp file, then rename),
/// so an interrupted save cannot leave a half-written credential behind.
pub fn save(slot: Slot, key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("ключ пустой".into());
    }
    if key.len() > MAX_KEY_BYTES {
        return Err("ключ слишком длинный".into());
    }
    if key.chars().any(|c| c.is_control()) {
        return Err("ключ содержит недопустимые символы".into());
    }

    let path = slot
        .file()
        .ok_or("не удалось определить каталог настроек")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("каталог настроек: {e}"))?;
    }

    let sealed = seal(key.as_bytes())?;
    let mut file = Vec::with_capacity(MAGIC.len() + sealed.len());
    file.extend_from_slice(MAGIC);
    file.extend_from_slice(&sealed);

    let tmp = path.with_extension("key.tmp");
    std::fs::write(&tmp, &file).map_err(|e| format!("запись ключа: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("переименование ключа: {e}")
    })?;
    Ok(())
}

/// Remove the stored key. A missing file counts as success.
pub fn clear(slot: Slot) -> Result<(), String> {
    let Some(path) = slot.file() else {
        return Ok(());
    };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("удаление ключа: {e}")),
    }
}

/// Read and decrypt the stored key. The `String` is the plaintext credential:
/// callers keep it in a local, hand it to the HTTP layer, and never log it.
pub fn load(slot: Slot) -> Result<String, String> {
    let path = slot
        .file()
        .ok_or("не удалось определить каталог настроек")?;
    let raw = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "ключ не сохранён".to_string(),
        _ => format!("чтение ключа: {e}"),
    })?;
    let body = raw
        .strip_prefix(MAGIC.as_slice())
        .ok_or("файл ключа повреждён")?;
    let plain = unseal(body)?;
    String::from_utf8(plain).map_err(|_| "файл ключа повреждён".to_string())
}

// ---------------------------------------------------------------------------
// Windows DPAPI (CryptProtectData / CryptUnprotectData), linked directly.
//
// Called through a two-field struct and two functions, so there is no reason to
// grow the dependency tree or to depend on a `windows` crate feature layout:
// the ABI here has been stable since Windows 2000.
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod dpapi {
    use std::ffi::c_void;

    /// `DATA_BLOB`: length in bytes plus a pointer to the bytes.
    #[repr(C)]
    pub struct DataBlob {
        pub cb_data: u32,
        pub pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        pub fn CryptProtectData(
            data_in: *const DataBlob,
            data_descr: *const u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        pub fn CryptUnprotectData(
            data_in: *const DataBlob,
            data_descr: *mut *mut u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        pub fn LocalFree(mem: *mut c_void) -> *mut c_void;
    }

    /// `CRYPTPROTECT_UI_FORBIDDEN`: never flash a Windows prompt at the user —
    /// this runs on a background poll thread.
    pub const UI_FORBIDDEN: u32 = 0x1;

    /// Copy the DPAPI output blob into a `Vec` and release it.
    ///
    /// # Safety
    /// `out` must be a blob filled in by a successful DPAPI call.
    pub unsafe fn take_blob(out: DataBlob) -> Vec<u8> {
        if out.pb_data.is_null() || out.cb_data == 0 {
            return Vec::new();
        }
        let bytes = std::slice::from_raw_parts(out.pb_data, out.cb_data as usize).to_vec();
        LocalFree(out.pb_data as *mut c_void);
        bytes
    }
}

#[cfg(windows)]
fn seal(plain: &[u8]) -> Result<Vec<u8>, String> {
    use dpapi::{take_blob, CryptProtectData, DataBlob, UI_FORBIDDEN};

    let input = DataBlob {
        cb_data: plain.len() as u32,
        pb_data: plain.as_ptr() as *mut u8,
    };
    let mut out = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    // No entropy and no description: the blob is only ever read back by this
    // app in this same user profile.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err(format!(
            "Windows не смогла зашифровать ключ ({})",
            std::io::Error::last_os_error()
        ));
    }
    Ok(unsafe { take_blob(out) })
}

#[cfg(windows)]
fn unseal(sealed: &[u8]) -> Result<Vec<u8>, String> {
    use dpapi::{take_blob, CryptUnprotectData, DataBlob, UI_FORBIDDEN};

    if sealed.is_empty() {
        return Err("файл ключа повреждён".into());
    }
    let input = DataBlob {
        cb_data: sealed.len() as u32,
        pb_data: sealed.as_ptr() as *mut u8,
    };
    let mut out = DataBlob {
        cb_data: 0,
        pb_data: std::ptr::null_mut(),
    };
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            UI_FORBIDDEN,
            &mut out,
        )
    };
    if ok == 0 {
        return Err("ключ не удалось расшифровать (сохранён другим пользователем?)".into());
    }
    Ok(unsafe { take_blob(out) })
}

#[cfg(not(windows))]
fn seal(_plain: &[u8]) -> Result<Vec<u8>, String> {
    Err("защищённое хранилище ключа доступно только в Windows".into())
}

#[cfg(not(windows))]
fn unseal(_sealed: &[u8]) -> Result<Vec<u8>, String> {
    Err("защищённое хранилище ключа доступно только в Windows".into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::{clear, exists, hint, load, save, Slot};

    /// The tests all use the same real per-user file, and cargo runs them in
    /// parallel: without this lock one test's store state leaks into another's
    /// assertions (and could clobber a developer's real key between two of them).
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// These tests touch the real per-user file, so each one restores whatever
    /// was stored before it ran — a developer with a key configured must not
    /// lose it because `cargo test` happened.
    fn with_clean_store(f: impl FnOnce()) {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let before = std::fs::read(Slot::DEEPSEEK.file().unwrap()).ok();
        clear(Slot::DEEPSEEK).unwrap();
        f();
        clear(Slot::DEEPSEEK).unwrap();
        if let Some(bytes) = before {
            let _ = std::fs::write(Slot::DEEPSEEK.file().unwrap(), bytes);
        }
    }

    #[test]
    fn round_trips_through_dpapi() {
        with_clean_store(|| {
            save(Slot::DEEPSEEK, "test-key-1234").unwrap();
            assert!(exists(Slot::DEEPSEEK));
            assert_eq!(load(Slot::DEEPSEEK).unwrap(), "test-key-1234");
        });
    }

    #[test]
    fn the_file_is_not_plaintext() {
        with_clean_store(|| {
            save(Slot::DEEPSEEK, "super-secret-marker").unwrap();
            let raw = std::fs::read(Slot::DEEPSEEK.file().unwrap()).unwrap();
            assert!(
                !raw.windows(6).any(|w| w == b"secret"),
                "ключ лежит в файле открытым текстом"
            );
        });
    }

    #[test]
    fn slots_are_separate_files() {
        with_clean_store(|| {
            save(Slot::DEEPSEEK, "deepseek-value").unwrap();
            assert_eq!(load(Slot::DEEPSEEK).unwrap(), "deepseek-value");
            assert_ne!(
                Slot::DEEPSEEK.file().unwrap(),
                Slot::MINIMAX.file().unwrap()
            );
        });
    }

    #[test]
    fn hint_shows_only_the_tail() {
        with_clean_store(|| {
            save(Slot::DEEPSEEK, "abcdefgh-9999").unwrap();
            assert_eq!(hint(Slot::DEEPSEEK).unwrap(), "9999");
        });
    }

    #[test]
    fn empty_and_control_input_is_rejected() {
        with_clean_store(|| {
            assert!(save(Slot::DEEPSEEK, "   ").is_err());
            assert!(save(Slot::DEEPSEEK, "bad\nkey").is_err());
        });
    }
}
