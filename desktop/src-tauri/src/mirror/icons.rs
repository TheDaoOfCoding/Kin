//! The cache of app icons that the phone sends (`notif.icon`, design 5.6): `<data dir>\notif-icons\<hash>.png`. **It is the only thing about
//! notifications that Kin keeps on disk**, and what it keeps is a picture of an app's icon, never anything from a notification.
//!
//! A PNG gets in only after the wire checks of `wire.rs` (base64url, at most 32,768 bytes, the hash, a square PNG of 24 to 128 pixels) **and** a decode
//! by the Windows Imaging Component, so that a PNG that is odd inside can't reach the shell's own image loader (the toast shows the file). The name
//! is the hash, which is hex only, so no path can be injected. A file that is there is never overwritten. At most 200 files or 8 MiB are kept: the
//! oldest by modification time go first (at start and every hour).
use std::path::{Path, PathBuf};

use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICImagingFactory, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::core::GUID;

use super::wire::{ICON_MAX_PIXELS, ICON_MIN_PIXELS, is_hex32};

pub const MAX_FILES: usize = 200;
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;
pub const FOLDER: &str = "notif-icons";

/// What a stored icon came to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stored {
    Written,
    AlreadyThere,
    Rejected(String),
}

/// Decides whether the bytes of a PNG are one the system can really decode as a square picture of 24 to 128 pixels. The real one is
/// [`WicChecker`]; a test can bring its own.
pub trait IconChecker: Send + Sync {
    fn check(&self, png: &[u8]) -> Result<(u32, u32), String>;
}

pub struct IconCache {
    dir: PathBuf,
}

impl IconCache {
    pub fn new(data_dir: &Path) -> IconCache {
        IconCache { dir: data_dir.join(FOLDER) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, hash: &str) -> Option<PathBuf> {
        is_hex32(hash).then(|| self.dir.join(format!("{hash}.png")))
    }

    /// The file of an icon, if it is in the cache.
    pub fn path_of(&self, hash: &str) -> Option<PathBuf> {
        self.file(hash).filter(|p| p.is_file())
    }

    /// Checks and stores an icon whose hash and PNG header the wire layer has verified. Blocking (a decode and a file write).
    pub fn store(&self, hash: &str, png: &[u8], checker: &dyn IconChecker) -> Stored {
        let Some(path) = self.file(hash) else {
            return Stored::Rejected("not a hash".into());
        };
        if path.is_file() {
            return Stored::AlreadyThere;
        }
        if let Err(e) = checker.check(png) {
            return Stored::Rejected(e);
        }
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            return Stored::Rejected(format!("the cache folder can't be made: {e}"));
        }
        let tmp = self.dir.join(format!("{hash}.tmp"));
        match std::fs::write(&tmp, png).and_then(|()| std::fs::rename(&tmp, &path)) {
            Ok(()) => Stored::Written,
            Err(e) => {
                let _ = std::fs::remove_file(&tmp);
                Stored::Rejected(format!("the file can't be written: {e}"))
            }
        }
    }

    /// Removes the oldest files while there are more than 200 or more than 8 MiB. Returns how many went.
    pub fn prune(&self) -> usize {
        self.prune_to(MAX_FILES, MAX_BYTES)
    }

    pub fn prune_to(&self, max_files: usize, max_bytes: u64) -> usize {
        let Ok(read) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = read
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                (m.is_file() && e.path().extension().is_some_and(|x| x == "png"))
                    .then(|| (m.modified().unwrap_or(std::time::UNIX_EPOCH), m.len(), e.path()))
            })
            .collect();
        files.sort();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        let mut count = files.len();
        let mut removed = 0;
        for (_, len, path) in files {
            if count <= max_files && total <= max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                removed += 1;
                count -= 1;
                total = total.saturating_sub(len);
            }
        }
        removed
    }

    /// The whole folder goes (when the last phone is unpaired, or the feature is turned off for good).
    pub fn remove_all(&self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }

    pub fn count(&self) -> usize {
        std::fs::read_dir(&self.dir).map_or(0, |r| r.flatten().filter(|e| e.path().extension().is_some_and(|x| x == "png")).count())
    }
}

/// COM for this thread, undone when it goes out of scope if it worked.
struct Com(bool);

impl Com {
    fn init() -> Com {
        // SAFETY: balanced by `CoUninitialize` in `Drop` when this succeeded.
        Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok())
    }
}

impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            // SAFETY: balances the successful `CoInitializeEx` of `init`.
            unsafe { CoUninitialize() };
        }
    }
}

/// The Windows Imaging Component as the judge of an icon: it must open the bytes as a PNG with one square frame of 24 to 128 pixels and produce its
/// pixels. (WIC is lenient with a PNG that is cut short or has a bad checksum, and fills what is missing: that is the same decoder the shell uses to
/// draw the toast, so what is accepted here is what would be drawn, never anything the shell would run.)
pub struct WicChecker;

impl IconChecker for WicChecker {
    fn check(&self, png: &[u8]) -> Result<(u32, u32), String> {
        let _com = Com::init();
        // SAFETY: COM calls on this thread. Every interface is released when it goes out of scope (before `_com`). The stream reads `png`, which
        // outlives it, and the pixel buffer is exactly `width * height * 4` bytes, as `CopyPixels` is told.
        unsafe {
            let factory: IWICImagingFactory =
                CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("no image decoder: {e}"))?;
            let stream = factory.CreateStream().map_err(|e| e.to_string())?;
            stream.InitializeFromMemory(png).map_err(|e| format!("the icon can't be read: {e}"))?;
            let decoder = factory
                .CreateDecoderFromStream(&stream, std::ptr::null::<GUID>(), WICDecodeMetadataCacheOnDemand)
                .map_err(|e| format!("not a picture Windows can read ({e})"))?;
            if decoder.GetContainerFormat().map_err(|e| e.to_string())? != GUID_ContainerFormatPng {
                return Err("not a PNG".into());
            }
            let frame = decoder.GetFrame(0).map_err(|e| format!("no picture in the file ({e})"))?;
            let (mut w, mut h) = (0u32, 0u32);
            frame.GetSize(&mut w, &mut h).map_err(|e| e.to_string())?;
            if w != h || !(ICON_MIN_PIXELS..=ICON_MAX_PIXELS).contains(&w) {
                return Err(format!("{w} x {h} pixels is not a square of {ICON_MIN_PIXELS} to {ICON_MAX_PIXELS}"));
            }
            let converter = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
            converter
                .Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
                .map_err(|e| format!("can't convert the picture ({e})"))?;
            let mut pixels = vec![0u8; (w * h * 4) as usize];
            converter.CopyPixels(std::ptr::null(), w * 4, &mut pixels).map_err(|e| format!("the picture is damaged ({e})"))?;
            Ok((w, h))
        }
    }
}

/// A small valid PNG of `w` x `h` pixels (one colour, RGBA, stored in uncompressed blocks), for tests and tools that need an icon.
#[doc(hidden)]
pub fn test_png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for b in data {
            c ^= u32::from(*b);
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    // Each row starts with the filter byte 0 ("none").
    let mut row = vec![0u8];
    for _ in 0..w {
        row.extend_from_slice(&rgba);
    }
    let raw: Vec<u8> = row.repeat(h as usize);
    // zlib: a header, stored blocks of at most 65,535 bytes, the Adler-32 of the data.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = raw.chunks(65_535).collect();
    for (k, block) in blocks.iter().enumerate() {
        z.push(u8::from(k + 1 == blocks.len()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for x in &raw {
        a = (a + u32::from(*x)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut png = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Accept;
    impl IconChecker for Accept {
        fn check(&self, _: &[u8]) -> Result<(u32, u32), String> {
            Ok((24, 24))
        }
    }
    struct Refuse;
    impl IconChecker for Refuse {
        fn check(&self, _: &[u8]) -> Result<(u32, u32), String> {
            Err("no".into())
        }
    }

    const HASH: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn an_icon_is_stored_by_its_hash_once_and_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IconCache::new(dir.path());
        assert_eq!(cache.path_of(HASH), None);
        assert_eq!(cache.store(HASH, b"first", &Accept), Stored::Written);
        let path = cache.path_of(HASH).unwrap();
        assert_eq!(path.file_name().unwrap(), format!("{HASH}.png").as_str());
        assert_eq!(cache.store(HASH, b"second", &Accept), Stored::AlreadyThere);
        assert_eq!(std::fs::read(&path).unwrap(), b"first");
        assert_eq!(std::fs::read_dir(cache.dir()).unwrap().count(), 1, "no temp file is left");
    }

    #[test]
    fn a_name_that_is_not_a_hash_never_reaches_the_file_system_and_a_refused_icon_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IconCache::new(dir.path());
        for bad in ["..\\..\\evil", "../x", "", "ABCDEF0123456789ABCDEF0123456789", &HASH[1..], "0123456789abcdef0123456789abcdez"] {
            assert!(matches!(cache.store(bad, b"x", &Accept), Stored::Rejected(_)), "{bad:?}");
            assert_eq!(cache.path_of(bad), None);
        }
        assert_eq!(cache.store(HASH, b"x", &Refuse), Stored::Rejected("no".into()));
        assert_eq!(cache.count(), 0);
        assert!(!dir.path().join("evil").exists());
    }

    #[test]
    fn the_oldest_files_go_first_when_there_are_too_many_or_too_big() {
        let dir = tempfile::tempdir().unwrap();
        let cache = IconCache::new(dir.path());
        std::fs::create_dir_all(cache.dir()).unwrap();
        let hashes: Vec<String> = (0..6).map(|k| format!("{k:032x}")).collect();
        for (k, h) in hashes.iter().enumerate() {
            let p = cache.dir().join(format!("{h}.png"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + k as u64 * 10);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        assert_eq!(cache.prune_to(4, u64::MAX), 2);
        assert!(cache.path_of(&hashes[0]).is_none() && cache.path_of(&hashes[1]).is_none() && cache.path_of(&hashes[2]).is_some());
        assert_eq!(cache.prune_to(100, 250), 2, "400 bytes left, 250 allowed: two more go");
        assert_eq!(cache.count(), 2);
        assert!(cache.path_of(&hashes[5]).is_some(), "the newest stays");
        cache.remove_all();
        assert_eq!(cache.count(), 0);
    }

    #[test]
    fn the_windows_decoder_accepts_a_square_icon_and_refuses_the_rest() {
        let ok = WicChecker.check(&test_png(24, 24, [200, 30, 30, 255]));
        assert_eq!(ok, Ok((24, 24)));
        assert_eq!(WicChecker.check(&test_png(96, 96, [20, 120, 220, 255])), Ok((96, 96)));
        assert_eq!(WicChecker.check(&test_png(128, 128, [1, 2, 3, 255])), Ok((128, 128)));
        assert!(WicChecker.check(&test_png(23, 23, [1, 2, 3, 255])).is_err());
        assert!(WicChecker.check(&test_png(129, 129, [1, 2, 3, 255])).is_err());
        assert!(WicChecker.check(&test_png(24, 48, [1, 2, 3, 255])).is_err());
        assert!(WicChecker.check(b"not a picture at all, just some text that is long enough").is_err());
    }
}
