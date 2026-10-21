//! Cover art for the phone (design 5.2): a session's thumbnail as a picture the wire accepts (a PNG or JPEG of at most 24,576 bytes, longest side at most 256
//! pixels, at least 16). A thumbnail that already fits (Edge's is a 2,419-byte PNG) goes through untouched; any other is decoded with WIC (its size is looked at
//! before any pixel), scaled down so the longest side is 256, and written as a JPEG, smaller (192, 128, 96, 64 pixels) until it fits. Nothing is kept on disk:
//! album art is personal.
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatJpeg, GUID_WICPixelFormat24bppBGR, IWICBitmapSource, IWICImagingFactory, IWICStream,
    WICBitmapDitherTypeNone, WICBitmapEncoderNoCache, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::CreateStreamOnHGlobal;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize, IStream, STATFLAG_NONAME, STATSTG,
    STREAM_SEEK_SET,
};
use windows::core::Interface;

use super::wire::{ART_MAX_PIXELS, IMAGE_MIN_PIXELS, ImageKind, MAX_IMAGE_BYTES, hash_of, image_fits};

/// A thumbnail whose sides are bigger than this is not decoded at all.
pub const MAX_SOURCE_PIXELS: u32 = 8192;

/// A picture ready for the wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Art {
    pub bytes: Vec<u8>,
    pub hash: String,
}

/// The sizes to try, largest first, for a source of `w` x `h`: the longest side 256 (or the source's own, if smaller), then 192, 128, 96 and 64 (each only if smaller
/// than the one before), keeping the shape. Both sides stay at least 16.
pub fn size_steps(w: u32, h: u32) -> Vec<(u32, u32)> {
    let longest = w.max(h).max(1);
    let mut sides = vec![ART_MAX_PIXELS.min(longest)];
    for s in [192, 128, 96, 64] {
        if s < *sides.last().expect("one side") {
            sides.push(s);
        }
    }
    sides
        .into_iter()
        .map(|side| {
            let scale = |x: u32| {
                ((u64::from(x) * u64::from(side) + u64::from(longest) / 2) / u64::from(longest)).max(u64::from(IMAGE_MIN_PIXELS)) as u32
            };
            (scale(w), scale(h))
        })
        .collect()
}

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

/// The picture to send for a session's thumbnail, or `None` (no art is fine: the next state just has none).
pub fn prepare(thumbnail: &[u8]) -> Option<Art> {
    if image_fits(thumbnail, ImageKind::Art) {
        return Some(Art { bytes: thumbnail.to_vec(), hash: hash_of(thumbnail) });
    }
    let _com = Com::init();
    match shrink(thumbnail) {
        Ok(art) => art,
        Err(e) => {
            log::info!("a cover picture could not be prepared: {e}");
            None
        }
    }
}

fn shrink(thumbnail: &[u8]) -> windows::core::Result<Option<Art>> {
    // SAFETY: COM calls on this thread; every interface is released before `Com` is dropped (it is declared first in the caller).
    unsafe {
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let stream: IWICStream = factory.CreateStream()?;
        stream.InitializeFromMemory(thumbnail)?;
        let decoder = factory.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h)?;
        if w < IMAGE_MIN_PIXELS || h < IMAGE_MIN_PIXELS || w > MAX_SOURCE_PIXELS || h > MAX_SOURCE_PIXELS {
            return Ok(None);
        }
        for (tw, th) in size_steps(w, h) {
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&frame, tw, th, WICBitmapInterpolationModeFant)?;
            let converter = factory.CreateFormatConverter()?;
            converter.Initialize(&scaler, &GUID_WICPixelFormat24bppBGR, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom)?;
            let bytes = encode_jpeg(&factory, &converter.cast::<IWICBitmapSource>()?, tw, th)?;
            if bytes.len() <= MAX_IMAGE_BYTES && image_fits(&bytes, ImageKind::Art) {
                let hash = hash_of(&bytes);
                return Ok(Some(Art { bytes, hash }));
            }
        }
        Ok(None)
    }
}

/// A JPEG of the source, in memory (WIC's default quality).
unsafe fn encode_jpeg(factory: &IWICImagingFactory, source: &IWICBitmapSource, w: u32, h: u32) -> windows::core::Result<Vec<u8>> {
    // SAFETY: the caller's contract: COM is initialised on this thread. The memory stream owns its block (`true`).
    unsafe {
        let stream: IStream = CreateStreamOnHGlobal(Default::default(), true)?;
        let encoder = factory.CreateEncoder(&GUID_ContainerFormatJpeg, std::ptr::null())?;
        encoder.Initialize(&stream, WICBitmapEncoderNoCache)?;
        let mut frame = None;
        encoder.CreateNewFrame(&mut frame, std::ptr::null_mut())?;
        let frame = frame.ok_or_else(windows::core::Error::empty)?;
        frame.Initialize(None)?;
        frame.SetSize(w, h)?;
        let mut format = GUID_WICPixelFormat24bppBGR;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
        let mut stat = STATSTG::default();
        stream.Stat(&mut stat, STATFLAG_NONAME)?;
        let size = stat.cbSize as usize;
        stream.Seek(0, STREAM_SEEK_SET, None)?;
        let mut bytes = vec![0u8; size];
        let mut read = 0u32;
        stream.Read(bytes.as_mut_ptr().cast(), size as u32, Some(&mut read)).ok()?;
        bytes.truncate(read as usize);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::wire::{Format, image_info};
    use windows::Win32::Graphics::Imaging::{GUID_ContainerFormatPng, GUID_WICPixelFormat24bppBGR as BGR};

    /// A picture of the given size made with WIC itself: a diagonal gradient with some noise, so a JPEG of it is not tiny.
    fn make(w: u32, h: u32, png: bool) -> Vec<u8> {
        let _com = Com::init();
        // SAFETY: COM calls on this thread; every interface is released before `_com`.
        unsafe {
            let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).unwrap();
            let stream: IStream = CreateStreamOnHGlobal(Default::default(), true).unwrap();
            let container = if png { GUID_ContainerFormatPng } else { GUID_ContainerFormatJpeg };
            let encoder = factory.CreateEncoder(&container, std::ptr::null()).unwrap();
            encoder.Initialize(&stream, WICBitmapEncoderNoCache).unwrap();
            let mut frame = None;
            encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()).unwrap();
            let frame = frame.unwrap();
            frame.Initialize(None).unwrap();
            frame.SetSize(w, h).unwrap();
            let mut format = BGR;
            frame.SetPixelFormat(&mut format).unwrap();
            let mut seed = 12345u32;
            for y in 0..h {
                let row: Vec<u8> = (0..w)
                    .flat_map(|x| {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        let noise = (seed >> 24) as u8 / 4;
                        [(x * 255 / w) as u8 ^ noise, (y * 255 / h) as u8, ((x + y) * 255 / (w + h)) as u8 ^ noise]
                    })
                    .collect();
                frame.WritePixels(1, w * 3, &row).unwrap();
            }
            frame.Commit().unwrap();
            encoder.Commit().unwrap();
            let mut stat = STATSTG::default();
            stream.Stat(&mut stat, STATFLAG_NONAME).unwrap();
            stream.Seek(0, STREAM_SEEK_SET, None).unwrap();
            let mut bytes = vec![0u8; stat.cbSize as usize];
            let mut read = 0u32;
            stream.Read(bytes.as_mut_ptr().cast(), bytes.len() as u32, Some(&mut read)).ok().unwrap();
            bytes.truncate(read as usize);
            bytes
        }
    }

    #[test]
    fn the_steps_start_at_256_keep_the_shape_and_stay_at_least_16() {
        assert_eq!(size_steps(640, 640), [(256, 256), (192, 192), (128, 128), (96, 96), (64, 64)]);
        assert_eq!(size_steps(1000, 500), [(256, 128), (192, 96), (128, 64), (96, 48), (64, 32)]);
        assert_eq!(size_steps(100, 100), [(100, 100), (96, 96), (64, 64)], "never larger than the source");
        assert_eq!(size_steps(2000, 20), [(256, 16), (192, 16), (128, 16), (96, 16), (64, 16)]);
        assert!(size_steps(640, 480).iter().all(|(w, h)| *w >= 16 && *h >= 16));
    }

    #[test]
    fn a_thumbnail_that_already_fits_goes_through_untouched() {
        let small = make(64, 64, true);
        assert!(small.len() <= MAX_IMAGE_BYTES);
        let art = prepare(&small).expect("fits");
        assert_eq!(art.bytes, small);
        assert_eq!(art.hash, hash_of(&small));
    }

    #[test]
    fn a_big_picture_comes_out_at_most_256_pixels_and_24576_bytes() {
        let big = make(1000, 800, false);
        assert!(big.len() > MAX_IMAGE_BYTES, "the source is bigger than the limit: {}", big.len());
        let art = prepare(&big).expect("shrunk");
        assert!(art.bytes.len() <= MAX_IMAGE_BYTES);
        let info = image_info(&art.bytes).expect("a picture");
        assert_eq!(info.format, Format::Jpeg);
        assert!(info.width.max(info.height) <= 256 && info.width >= 16 && info.height >= 16, "{info:?}");
        assert!(image_fits(&art.bytes, ImageKind::Art));
        assert_eq!(art.hash, hash_of(&art.bytes));
        // The shape is kept: 1000 x 800 is 5:4.
        assert!((i64::from(info.width) * 4 - i64::from(info.height) * 5).abs() <= 8, "{info:?}");
    }

    #[test]
    fn a_picture_that_is_not_one_gives_no_art() {
        assert!(prepare(b"this is text, not a picture").is_none());
        assert!(prepare(&[]).is_none());
        let tiny = make(8, 8, true);
        assert!(prepare(&tiny).is_none(), "under 16 pixels");
    }
}
