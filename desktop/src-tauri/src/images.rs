//! A received image on the Windows clipboard (windows.md, "Receiving from the phone").
//!
//! **What goes on the clipboard, and why.** An image that arrives from the phone is put on the clipboard in the formats that make
//! Ctrl+V work in the apps people paste into:
//!
//! * **`CF_DIBV5`**, a 32-bit BGRA bitmap with an alpha channel. Windows builds `CF_DIB` and `CF_BITMAP` from it on demand, so Paint,
//!   Word, Outlook, Photoshop and every other app that asks for a plain bitmap get one (an app that wants alpha gets alpha).
//! * **`PNG`**, the registered format with the original PNG bytes, when the file *is* a PNG (the decoder says so: the extension and the
//!   type the phone gave are not trusted): Chrome, Edge, Firefox, Teams, Slack and Office prefer it, and it keeps the file exactly as it was.
//! * **`CF_HDROP`** with the path of the file in the received folder: Explorer pastes the file, and chat apps attach the original
//!   file (the full-quality photo, with its name) instead of a bitmap.
//!
//! **What is an image is decided by the content**: a file is a picture if the Windows Imaging Component (WIC) can open it and finds a
//! frame in it (PNG, JPEG, GIF's first frame, BMP, TIFF, and WebP/HEIC/AVIF where the user has the codec), whatever its name or the
//! MIME type the phone sent says.
//!
//! **Rotation without the WIC rotator.** A phone photo has its *rotation in the EXIF data* and not in the pixels, and an app that reads a
//! bitmap never looks at EXIF, so the rotation is applied here. The WIC flip-rotator does it by asking its source for one scanline per
//! pixel when the source is a decoder, which took over 400 s for a 12 MP photo (measured on the real app, 1.3 review). So the picture is
//! read **row by row, top to bottom, in order** (the one access pattern a JPEG decoder is fast at), and each pixel is written straight to
//! its place in the bottom-up DIB: no rotator, no vertical flip, no second copy of the picture ([`place_row`]).
//!
//! **Memory is bounded.** The pixels are written straight into the one block of memory that becomes the clipboard's `CF_DIBV5`,
//! 4 bytes a pixel, and that block starts zeroed (nothing uninitialised is ever read). An image that needs more than
//! [`MAX_PIXEL_BYTES`] (64 MiB: about 16 megapixels, which covers a 12 MP phone photo) is not put on the clipboard, and the log says why.
//! The `PNG` format is skipped for a file over 64 MiB the same way.
use std::path::{Path, PathBuf};

use windows::Win32::Foundation::{GENERIC_READ, GlobalFree, HANDLE, HGLOBAL, HWND};
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA, IWICBitmapFrameDecode, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand, WICRect,
};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantClear};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW, SetClipboardData};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Variant::VT_UI2;
use windows::core::{HSTRING, w};

use crate::Result;
use crate::clipboard::ImageOutcome;
use crate::paths::long_path;

/// The most memory the pixels of an image may take (4 bytes a pixel).
pub const MAX_PIXEL_BYTES: u64 = 64 * 1024 * 1024;
const CF_DIBV5: u32 = 17;
const CF_HDROP: u32 = 15;
/// `BITMAPV5HEADER` is 124 bytes.
const V5_HEADER: usize = 124;
/// The picture is read in bands of about this many bytes (a few hundred rows of a phone photo).
const BAND_BYTES: usize = 4 << 20;
/// A file bigger than this is not looked at when the received files are searched for a picture (its pixels would not fit anyway).
const SNIFF_MAX_FILE_BYTES: u64 = 1 << 30;

/// What the decoder found out about an image, before any pixels are read.
#[derive(Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// The container of an image, as the decoder reports it (not as the name says).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageKind {
    Png,
    Other,
}

/// The bytes of memory the pixels of an image of this size need, or `None` for a size that can't be a picture.
pub fn pixel_bytes(width: u32, height: u32) -> Option<u64> {
    if width == 0 || height == 0 {
        return None;
    }
    u64::from(width).checked_mul(u64::from(height))?.checked_mul(4)
}

/// Whether an image of this size may go on the clipboard.
pub fn fits(width: u32, height: u32) -> bool {
    pixel_bytes(width, height).is_some_and(|b| b <= MAX_PIXEL_BYTES)
}

/// The size of the picture once it is upright: the sides swap for the orientations that turn it a quarter (5 to 8).
pub fn upright_size(orientation: u16, width: u32, height: u32) -> (u32, u32) {
    if (5..=8).contains(&orientation) { (height, width) } else { (width, height) }
}

/// Where the pixel `(x, y)` of a stored `width` x `height` picture is in the upright picture (top-down, from the top left), for an EXIF
/// orientation (1 to 8; anything else is 1). 2 mirrors left to right, 3 turns half a turn, 4 mirrors top to bottom, 5 is the
/// transpose, 6 turns a quarter clockwise (a phone held upright, stored sideways), 7 is the transverse and 8 turns a quarter back.
pub fn upright_position(orientation: u16, width: u32, height: u32, x: u32, y: u32) -> (u32, u32) {
    match orientation {
        2 => (width - 1 - x, y),
        3 => (width - 1 - x, height - 1 - y),
        4 => (x, height - 1 - y),
        5 => (y, x),
        6 => (height - 1 - y, x),
        7 => (height - 1 - y, width - 1 - x),
        8 => (y, width - 1 - x),
        _ => (x, y),
    }
}

/// Writes row `y` of the stored picture (`row`, `width` pixels) into `dst`, the pixels of a **bottom-up** DIB of the upright picture
/// (`upright_size`): each pixel goes where [`upright_position`] says, and the last row of the picture is the first row in memory.
/// The rotations are written out so that a 12 MP picture is a few tens of milliseconds, not a call per pixel.
pub fn place_row(orientation: u16, width: u32, height: u32, y: u32, row: &[u32], dst: &mut [u32]) {
    let (w, h) = (width as usize, height as usize);
    let y = y as usize;
    let (dw, dh) = (upright_size(orientation, width, height).0 as usize, upright_size(orientation, width, height).1 as usize);
    debug_assert!(row.len() == w && dst.len() == dw * dh);
    // The row of the DIB that holds upright row `dy`: bottom-up.
    let dib_row = |dy: usize| (dh - 1 - dy) * dw;
    match orientation {
        2 => {
            let out = &mut dst[dib_row(y)..][..dw];
            for (o, p) in out.iter_mut().zip(row.iter().rev()) {
                *o = *p;
            }
        }
        3 => dst[dib_row(h - 1 - y)..][..dw].iter_mut().zip(row.iter().rev()).for_each(|(o, p)| *o = *p),
        4 => dst[dib_row(h - 1 - y)..][..dw].copy_from_slice(row),
        // The four that turn the picture a quarter: a column of the upright picture is a row of the stored one.
        5 => row.iter().enumerate().for_each(|(x, p)| dst[dib_row(x) + y] = *p),
        6 => row.iter().enumerate().for_each(|(x, p)| dst[dib_row(x) + (h - 1 - y)] = *p),
        7 => row.iter().enumerate().for_each(|(x, p)| dst[dib_row(w - 1 - x) + (h - 1 - y)] = *p),
        8 => row.iter().enumerate().for_each(|(x, p)| dst[dib_row(w - 1 - x) + y] = *p),
        _ => dst[dib_row(y)..][..dw].copy_from_slice(row),
    }
}

/// The EXIF orientation of a frame (1 when it has none, or the format has no metadata of that kind).
fn exif_orientation(frame: &IWICBitmapFrameDecode) -> u16 {
    let Ok(reader) = (unsafe { frame.GetMetadataQueryReader() }) else {
        return 1;
    };
    // JPEG keeps it in APP1, TIFF and (with an eXIf chunk) PNG at the top.
    for query in [w!("/app1/ifd/{ushort=274}"), w!("/ifd/{ushort=274}")] {
        let mut value = PROPVARIANT::default();
        // SAFETY: the query is a valid wide string and `value` is a valid place for the answer. The union is read only when its type
        // says it holds an unsigned 16-bit integer, and the value is cleared afterwards.
        let found = unsafe {
            let ok = reader.GetMetadataByName(query, &mut value).is_ok();
            let inner = &value.Anonymous.Anonymous;
            let n = (ok && inner.vt == VT_UI2).then(|| inner.Anonymous.uiVal);
            let _ = PropVariantClear(&mut value);
            n
        };
        if let Some(n) = found.filter(|n| (1..=8).contains(n)) {
            return n;
        }
    }
    1
}

/// COM for this thread, undone when it goes out of scope if it worked (a thread that already has COM in another mode is fine too).
/// Declare it *before* any interface, so the interfaces are released first.
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

/// A block of global memory that is freed unless it is handed to the clipboard. It starts zeroed.
struct Global(Option<HGLOBAL>);

impl Global {
    fn alloc(bytes: usize) -> Result<Global> {
        // SAFETY: plain allocation; the handle is freed in `Drop` unless `take` gave it away. `GMEM_ZEROINIT` makes every byte defined.
        Ok(Global(Some(unsafe { GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)? })))
    }

    fn handle(&self) -> HGLOBAL {
        self.0.expect("not given away yet")
    }

    fn take(mut self) -> HGLOBAL {
        self.0.take().expect("not given away yet")
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            // SAFETY: the block belongs to us (it was never given to the clipboard).
            let _ = unsafe { GlobalFree(Some(h)) };
        }
    }
}

/// The 124-byte `BITMAPV5HEADER` of a 32-bit BGRA bottom-up bitmap with an alpha channel (`BI_BITFIELDS`, sRGB).
fn v5_header(width: u32, height: u32, image_bytes: u32) -> [u8; V5_HEADER] {
    fn put(h: &mut [u8], at: usize, value: u32) {
        h[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    let mut h = [0u8; V5_HEADER];
    put(&mut h, 0, V5_HEADER as u32); // bV5Size
    put(&mut h, 4, width); // bV5Width
    put(&mut h, 8, height); // bV5Height: positive, so the rows are bottom-up
    h[12..14].copy_from_slice(&1u16.to_le_bytes()); // bV5Planes
    h[14..16].copy_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    put(&mut h, 16, 3); // bV5Compression: BI_BITFIELDS
    put(&mut h, 20, image_bytes); // bV5SizeImage
    put(&mut h, 40, 0x00FF_0000); // red mask
    put(&mut h, 44, 0x0000_FF00); // green mask
    put(&mut h, 48, 0x0000_00FF); // blue mask
    put(&mut h, 52, 0xFF00_0000); // alpha mask
    put(&mut h, 56, 0x7352_4742); // bV5CSType: LCS_sRGB ('sRGB')
    put(&mut h, 108, 4); // bV5Intent: LCS_GM_IMAGES
    h
}

/// A picture ready for the clipboard.
struct Decoded {
    /// The `CF_DIBV5` block: the header, then the pixels, bottom-up, upright.
    block: Global,
    width: u32,
    height: u32,
    kind: ImageKind,
}

/// Whether the decoder opens the file as a picture, and what container it is in. `None` for anything WIC can't read, and for a file that
/// has no frame or an empty one: a name or a MIME type never makes a file a picture.
pub fn sniff(path: &Path) -> Option<ImageKind> {
    let _com = Com::init();
    // SAFETY: COM calls on this thread; every interface is released when it goes out of scope (before `_com`).
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let decoder = factory
            .CreateDecoderFromFilename(&HSTRING::from(long_path(path).as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        if w == 0 || h == 0 {
            return None;
        }
        Some(if decoder.GetContainerFormat().ok()? == GUID_ContainerFormatPng { ImageKind::Png } else { ImageKind::Other })
    }
}

/// The files of `paths` that are pictures by their content, at most `limit` of them (the search stops there). Blocking: run it on the
/// blocking pool.
pub fn find_images(paths: &[PathBuf], limit: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for path in paths {
        let fits_a_look = std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0 && m.len() <= SNIFF_MAX_FILE_BYTES);
        if fits_a_look && sniff(path).is_some() {
            found.push(path.clone());
            if found.len() >= limit {
                break;
            }
        }
    }
    found
}

/// Decodes `path` into a ready-made `CF_DIBV5` block, upright. `Err(reason)` is "not an image the system can decode" or "too big".
fn decode(path: &Path) -> std::result::Result<Decoded, String> {
    let _com = Com::init();
    // SAFETY: COM calls on this thread. Every interface is released when it goes out of scope (before `_com`). The pixel block is
    // locked while it is written, is exactly the size that was allocated, and is unlocked before it is used.
    unsafe {
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).map_err(|e| format!("no image decoder: {e}"))?;
        let decoder = factory
            .CreateDecoderFromFilename(&HSTRING::from(long_path(path).as_os_str()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand)
            .map_err(|e| format!("not an image Windows can read ({e})"))?;
        let kind = if decoder.GetContainerFormat().is_ok_and(|g| g == GUID_ContainerFormatPng) { ImageKind::Png } else { ImageKind::Other };
        let frame = decoder.GetFrame(0).map_err(|e| format!("no picture in the file ({e})"))?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).map_err(|e| e.to_string())?;
        if !fits(w, h) {
            return Err(format!(
                "the image is {w} x {h} pixels, too big to put on the clipboard (the limit is {} MiB of pixels)",
                MAX_PIXEL_BYTES >> 20
            ));
        }
        let orientation = exif_orientation(&frame);
        let (dw, dh) = upright_size(orientation, w, h);
        let pixels = pixel_bytes(dw, dh).filter(|b| *b <= MAX_PIXEL_BYTES).ok_or("the image is too big")? as usize;

        // To 32-bit BGRA. The converter reads its source row by row; asking for the rows in order is what a JPEG decoder is fast at.
        let converter = factory.CreateFormatConverter().map_err(|e| e.to_string())?;
        converter
            .Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)
            .map_err(|e| format!("can't convert the picture ({e})"))?;

        let block = Global::alloc(V5_HEADER + pixels).map_err(|e| e.to_string())?;
        let ptr = GlobalLock(block.handle()) as *mut u8;
        if ptr.is_null() {
            return Err("the memory for the picture is not available".to_string());
        }
        std::ptr::copy_nonoverlapping(v5_header(dw, dh, pixels as u32).as_ptr(), ptr, V5_HEADER);
        // SAFETY (the slice): the block is `V5_HEADER + pixels` bytes and zeroed, the header is a multiple of 4 bytes and the block is
        // aligned to at least 8, so this is `dw * dh` aligned, initialised `u32`s inside the block.
        let dst = std::slice::from_raw_parts_mut(ptr.add(V5_HEADER) as *mut u32, (dw as usize) * (dh as usize));
        let band_rows = (BAND_BYTES / (w as usize * 4)).clamp(1, h as usize);
        let mut band = vec![0u32; w as usize * band_rows];
        let mut y = 0u32;
        let mut failed = None;
        while y < h {
            let rows = band_rows.min((h - y) as usize);
            let rect = WICRect { X: 0, Y: y as i32, Width: w as i32, Height: rows as i32 };
            let bytes = std::slice::from_raw_parts_mut(band.as_mut_ptr() as *mut u8, w as usize * rows * 4);
            if let Err(e) = converter.CopyPixels(&rect, w * 4, bytes) {
                failed = Some(e);
                break;
            }
            for r in 0..rows {
                place_row(orientation, w, h, y + r as u32, &band[r * w as usize..(r + 1) * w as usize], dst);
            }
            y += rows as u32;
        }
        let _ = GlobalUnlock(block.handle());
        if let Some(e) = failed {
            return Err(format!("can't read the pixels ({e})"));
        }
        Ok(Decoded { block, width: dw, height: dh, kind })
    }
}

/// Global memory with these bytes in it, for `SetClipboardData`.
fn block_of(bytes: &[u8]) -> Result<Global> {
    let block = Global::alloc(bytes.len().max(1))?;
    // SAFETY: the block is locked while it is written and was allocated with this size.
    unsafe {
        let ptr = GlobalLock(block.handle()) as *mut u8;
        if ptr.is_null() {
            return Err("GlobalLock failed".into());
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(block.handle());
    }
    Ok(block)
}

/// A `DROPFILES` block for one file: the 20-byte header (the list starts at byte 20, no point, not non-client, wide characters) and
/// the path as UTF-16 with two NULs at the end.
fn drop_files(path: &Path) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&20u32.to_le_bytes()); // pFiles
    bytes.extend_from_slice(&[0u8; 8]); // pt
    bytes.extend_from_slice(&0u32.to_le_bytes()); // fNC
    bytes.extend_from_slice(&1u32.to_le_bytes()); // fWide
    use std::os::windows::ffi::OsStrExt;
    for unit in path.as_os_str().encode_wide().chain([0, 0]) {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    bytes
}

/// Puts the image in `path` on the clipboard (see the module comment for what is put there). `owner` is the clipboard window.
pub fn write_clipboard_image(owner: HWND, path: &Path) -> Result<ImageOutcome> {
    let (dib, width, height, kind) = match decode(path) {
        Ok(d) => (d.block, d.width, d.height, d.kind),
        Err(why) => return Ok(ImageOutcome::Skipped(why)),
    };
    // The original PNG bytes, if the decoder says it is a PNG and it isn't huge.
    let png = if kind == ImageKind::Png && std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_PIXEL_BYTES) {
        std::fs::read(path).ok()
    } else {
        None
    };
    let png_block = png.as_deref().map(block_of).transpose()?;
    let drop_block = block_of(&drop_files(path))?;

    // The clipboard may be in use by another app for a moment.
    let mut opened = false;
    for attempt in 0..10 {
        // SAFETY: `owner` is a live window of this process.
        if unsafe { OpenClipboard(Some(owner)) }.is_ok() {
            opened = true;
            break;
        }
        if attempt < 9 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    if !opened {
        return Err("the clipboard is in use by another app".into());
    }
    let result = (|| -> Result<()> {
        // SAFETY: the clipboard is open. A block that SetClipboardData accepts belongs to the clipboard, so it is taken out of its guard.
        unsafe {
            EmptyClipboard()?;
            SetClipboardData(CF_DIBV5, Some(HANDLE(dib.take().0)))?;
            if let Some(block) = png_block {
                SetClipboardData(RegisterClipboardFormatW(w!("PNG")), Some(HANDLE(block.take().0)))?;
            }
            SetClipboardData(CF_HDROP, Some(HANDLE(drop_block.take().0)))?;
        }
        Ok(())
    })();
    // SAFETY: closes what was opened above.
    let _ = unsafe { CloseClipboard() };
    result?;
    Ok(ImageOutcome::Set { width, height })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::GENERIC_WRITE;
    use windows::Win32::Graphics::Imaging::{GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR, WICBitmapEncoderNoCache};

    use super::*;

    #[test]
    fn the_memory_limit_is_16_megapixels_of_32_bit_pixels() {
        assert!(fits(4000, 3000), "a 12 MP phone photo fits");
        assert!(fits(4096, 4096), "16.7 MP is the limit");
        assert!(!fits(4096, 4097));
        assert!(!fits(8000, 6000), "a 48 MP photo does not");
        assert!(!fits(0, 100) && !fits(100, 0));
        assert_eq!(pixel_bytes(u32::MAX, u32::MAX), None, "no overflow, and not something that fits");
        assert_eq!(pixel_bytes(2, 3), Some(24));
    }

    #[test]
    fn the_v5_header_describes_a_32_bit_bottom_up_bitmap_with_alpha() {
        let h = v5_header(3, 2, 24);
        let word = |at: usize| u32::from_le_bytes(h[at..at + 4].try_into().unwrap());
        assert_eq!((word(0), word(4), word(8), word(20)), (124, 3, 2, 24));
        assert_eq!(u16::from_le_bytes([h[14], h[15]]), 32);
        assert_eq!((word(16), word(40), word(44), word(48), word(52)), (3, 0xFF0000, 0xFF00, 0xFF, 0xFF00_0000));
        assert_eq!(&h[56..60], b"BGRs", "'sRGB' read as little-endian text");
    }

    #[test]
    fn the_drop_files_block_lists_one_wide_path_with_two_nuls() {
        let bytes = drop_files(Path::new("C:\\a.png"));
        assert_eq!(u32::from_le_bytes(bytes[..4].try_into().unwrap()), 20);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1, "wide");
        let units: Vec<u16> = bytes[20..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&units[..units.len() - 2]).unwrap(), "C:\\a.png");
        assert_eq!(&units[units.len() - 2..], &[0, 0]);
    }

    /// The picture a DIB holds, top row first.
    fn top_down(dib: &[u32], dw: usize, dh: usize) -> Vec<Vec<u32>> {
        (0..dh).map(|y| dib[(dh - 1 - y) * dw..][..dw].to_vec()).collect()
    }

    /// A stored 3 x 2 picture, `a b c / d e f`, run through [`place_row`] for one orientation.
    fn placed(orientation: u16) -> Vec<Vec<u32>> {
        let stored: [[u32; 3]; 2] = [[1, 2, 3], [4, 5, 6]]; // a b c / d e f
        let (dw, dh) = upright_size(orientation, 3, 2);
        let mut dib = vec![0xDEAD_BEEFu32; (dw * dh) as usize];
        for (y, row) in stored.iter().enumerate() {
            place_row(orientation, 3, 2, y as u32, row, &mut dib);
        }
        top_down(&dib, dw as usize, dh as usize)
    }

    #[test]
    fn each_orientation_turns_the_picture_upright_and_the_dib_is_bottom_up() {
        // What the upright picture looks like, top row first. (6: a phone held upright, stored sideways: turn a quarter clockwise.)
        let expected: [(u16, Vec<Vec<u32>>); 9] = [
            (0, vec![vec![1, 2, 3], vec![4, 5, 6]]),
            (1, vec![vec![1, 2, 3], vec![4, 5, 6]]),
            (2, vec![vec![3, 2, 1], vec![6, 5, 4]]),
            (3, vec![vec![6, 5, 4], vec![3, 2, 1]]),
            (4, vec![vec![4, 5, 6], vec![1, 2, 3]]),
            (5, vec![vec![1, 4], vec![2, 5], vec![3, 6]]),
            (6, vec![vec![4, 1], vec![5, 2], vec![6, 3]]),
            (7, vec![vec![6, 3], vec![5, 2], vec![4, 1]]),
            (8, vec![vec![3, 6], vec![2, 5], vec![1, 4]]),
        ];
        for (orientation, want) in expected {
            assert_eq!(placed(orientation), want, "orientation {orientation}");
        }
        assert_eq!(placed(9), placed(1), "an unknown orientation is upright");
    }

    #[test]
    fn upright_position_agrees_with_place_row_for_every_pixel_of_every_orientation() {
        let (w, h) = (5u32, 3u32);
        let stored: Vec<Vec<u32>> = (0..h).map(|y| (0..w).map(|x| y * 10 + x).collect()).collect();
        for orientation in 1..=8u16 {
            let (dw, dh) = upright_size(orientation, w, h);
            let mut dib = vec![u32::MAX; (dw * dh) as usize];
            for (y, row) in stored.iter().enumerate() {
                place_row(orientation, w, h, y as u32, row, &mut dib);
            }
            let upright = top_down(&dib, dw as usize, dh as usize);
            for y in 0..h {
                for x in 0..w {
                    let (ux, uy) = upright_position(orientation, w, h, x, y);
                    assert_eq!(upright[uy as usize][ux as usize], y * 10 + x, "orientation {orientation}, pixel {x},{y}");
                }
            }
            assert!(dib.iter().all(|p| *p != u32::MAX), "orientation {orientation}: every pixel of the DIB was written");
        }
    }

    // ----- real files, through WIC ----------------------------------------------------------------------------------------

    /// Red top left, green top right, blue bottom left, yellow bottom right (BGR bytes).
    fn quadrant(x: u32, y: u32, w: u32, h: u32) -> [u8; 3] {
        match (x < w / 2, y < h / 2) {
            (true, true) => [0, 0, 255],
            (false, true) => [0, 255, 0],
            (true, false) => [255, 0, 0],
            (false, false) => [0, 255, 255],
        }
    }

    /// Writes a JPEG of `w` x `h` pixels (the four quadrants) with the EXIF orientation tag `orientation`, made with WIC's own encoder.
    fn write_test_jpeg(path: &Path, w: u32, h: u32, orientation: u16) {
        let _com = Com::init();
        // SAFETY: COM calls on this thread; every interface is released before `_com`.
        unsafe {
            let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let stream = factory.CreateStream().unwrap();
            stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0).unwrap();
            let encoder = factory.CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null()).unwrap();
            encoder.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(None).unwrap();
            frame.SetSize(w, h).unwrap();
            let mut format = GUID_WICPixelFormat24bppBGR;
            frame.SetPixelFormat(&mut format).unwrap();
            if orientation != 1 {
                let writer = frame.GetMetadataQueryWriter().unwrap();
                writer.SetMetadataByName(w!("/app1/ifd/{ushort=274}"), &PROPVARIANT::from(orientation)).unwrap();
            }
            // Every row of the top half is the same, and so is every row of the bottom half.
            let row_of = |y: u32| -> Vec<u8> { (0..w).flat_map(|x| quadrant(x, y, w, h)).collect() };
            let (top, bottom) = (row_of(0), row_of(h - 1));
            let band = 256u32;
            let mut y = 0;
            while y < h {
                let rows = band.min(h - y);
                let mut bytes = Vec::with_capacity((rows * w * 3) as usize);
                for r in 0..rows {
                    bytes.extend_from_slice(if y + r < h / 2 { &top } else { &bottom });
                }
                frame.WritePixels(rows, w * 3, &bytes).unwrap();
                y += rows;
            }
            frame.Commit().unwrap();
            encoder.Commit().unwrap();
        }
    }

    /// The pixels of a decoded picture, top row first.
    fn pixels_of(d: &Decoded) -> Vec<Vec<u32>> {
        let (w, h) = (d.width as usize, d.height as usize);
        let mut all = vec![0u32; w * h];
        // SAFETY: the block holds the header and `w * h` pixels; it is locked while it is copied and the copy is of exactly that size.
        unsafe {
            let ptr = GlobalLock(d.block.handle()) as *const u8;
            assert!(!ptr.is_null());
            std::ptr::copy_nonoverlapping(ptr.add(V5_HEADER) as *const u32, all.as_mut_ptr(), w * h);
            let _ = GlobalUnlock(d.block.handle());
        }
        top_down(&all, w, h)
    }

    /// The colour a corner shows: 0 red, 1 green, 2 blue, 3 yellow (a pixel `inset` in from the corner, so JPEG's edges don't matter).
    fn corner_colours(pixels: &[Vec<u32>]) -> [char; 4] {
        let (h, w, inset) = (pixels.len(), pixels[0].len(), 40);
        let name = |p: u32| {
            let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
            match (r > 200, g > 200, b > 200) {
                (true, false, false) => 'r',
                (false, true, false) => 'g',
                (false, false, true) => 'b',
                (true, true, false) => 'y',
                _ => '?',
            }
        };
        // top left, top right, bottom right, bottom left
        [
            name(pixels[inset][inset]),
            name(pixels[inset][w - 1 - inset]),
            name(pixels[h - 1 - inset][w - 1 - inset]),
            name(pixels[h - 1 - inset][inset]),
        ]
    }

    /// The time limit for a 12 MP picture: the real one is under two seconds in a release build (about 0.1 s measured); a debug build
    /// (no optimisation of the pixel loops) gets a generous one. Before the fix this took over 400 s.
    fn limit() -> Duration {
        if cfg!(debug_assertions) { Duration::from_secs(30) } else { Duration::from_secs(2) }
    }

    #[test]
    fn a_12_megapixel_photo_with_exif_rotation_is_decoded_upright_within_the_time_limit() {
        let dir = tempfile::tempdir().unwrap();
        // 4000 x 3000 stored, so a quarter turn gives 3000 x 4000. Corners of the upright picture, clockwise from the top left.
        let cases: [(u16, (u32, u32), [char; 4]); 4] = [
            (1, (4000, 3000), ['r', 'g', 'y', 'b']),
            (6, (3000, 4000), ['b', 'r', 'g', 'y']), // turned a quarter clockwise: the stored bottom left is now the top left
            (3, (4000, 3000), ['y', 'b', 'r', 'g']),
            (8, (3000, 4000), ['g', 'y', 'b', 'r']),
        ];
        for (orientation, (uw, uh), corners) in cases {
            let path = dir.path().join(format!("photo-{orientation}.jpg"));
            write_test_jpeg(&path, 4000, 3000, orientation);
            assert!(std::fs::metadata(&path).unwrap().len() > 10_000);
            let started = Instant::now();
            let decoded = decode(&path).expect("a JPEG");
            let took = started.elapsed();
            eprintln!("12 MP JPEG, orientation {orientation}: decoded in {took:?}");
            assert!(took < limit(), "orientation {orientation} took {took:?}, the limit is {:?}", limit());
            assert_eq!((decoded.width, decoded.height), (uw, uh), "orientation {orientation}");
            assert_eq!(decoded.kind, ImageKind::Other);
            assert_eq!(corner_colours(&pixels_of(&decoded)), corners, "orientation {orientation}");
        }
    }

    #[test]
    fn what_is_a_picture_is_decided_by_the_content_and_not_by_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = dir.path().join("really-a-jpeg.txt");
        write_test_jpeg(&jpeg, 64, 48, 1);
        assert_eq!(sniff(&jpeg), Some(ImageKind::Other), "a JPEG named .txt is a picture");
        let not = dir.path().join("photo.jpg");
        std::fs::write(&not, "this is text, whatever the name says").unwrap();
        assert_eq!(sniff(&not), None, "text named .jpg is not");
        let empty = dir.path().join("empty.png");
        std::fs::write(&empty, "").unwrap();
        assert_eq!(sniff(&empty), None);
        assert_eq!(sniff(&dir.path().join("missing.png")), None);
        let png = dir.path().join("plain.dat");
        std::fs::write(&png, TINY_PNG).unwrap();
        assert_eq!(sniff(&png), Some(ImageKind::Png), "a PNG is a PNG whatever its name: that picks the PNG clipboard format");
        // The search finds both pictures and stops at the limit.
        let files = vec![not.clone(), jpeg.clone(), png.clone(), empty];
        assert_eq!(find_images(&files, 5), vec![jpeg.clone(), png.clone()]);
        assert_eq!(find_images(&files, 1), vec![jpeg]);
    }

    /// A 1 x 1 PNG.
    const TINY_PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00,
        0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63,
        0xFC, 0xCF, 0xC0, 0x50, 0x0F, 0x00, 0x04, 0x85, 0x01, 0x80, 0x84, 0xA9, 0x8C, 0x21, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
        0xAE, 0x42, 0x60, 0x82,
    ];

    #[test]
    fn an_image_that_is_too_big_is_refused_before_any_pixel_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.jpg");
        write_test_jpeg(&path, 4096, 4097, 1);
        let started = Instant::now();
        let why = decode(&path).err().expect("one pixel row over 16 MP is over the limit");
        assert!(why.contains("too big"), "{why}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
