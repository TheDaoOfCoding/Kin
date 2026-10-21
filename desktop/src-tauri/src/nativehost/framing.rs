//! The native messaging framing, used on the pipe to the host (docs/extension.md, "Framing" and "Pipe"): a 32-bit little-endian length, then
//! that many bytes of UTF-8 JSON, in both directions.
use std::io::{self, Read, Write};

/// A request on the pipe: the extension's (at most 64 MiB) plus the two fields the host adds, so 64 MiB + 1 MiB.
pub const MAX_REQUEST: usize = 64 * 1024 * 1024 + 1024 * 1024;
/// An answer: the host may send the browser at most 1 MiB, and it answers `INTERNAL` for more, so Kin never writes more.
pub const MAX_RESPONSE: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// The stream ended inside a header or a body.
    Truncated,
    /// The length prefix is over the limit (the body is not read).
    TooLarge(u64),
    Io(io::Error),
}

/// Reads one frame. `Ok(None)` is a clean end of stream before the first byte.
pub fn read_frame<R: Read>(r: &mut R, max: usize) -> Result<Option<Vec<u8>>, FrameError> {
    let mut head = [0u8; 4];
    let mut got = 0;
    while got < head.len() {
        match r.read(&mut head[got..]) {
            Ok(0) => return if got == 0 { Ok(None) } else { Err(FrameError::Truncated) },
            Ok(n) => got += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_le_bytes(head) as u64;
    if len > max as u64 {
        return Err(FrameError::TooLarge(len));
    }
    // Not `with_capacity(len)`: a hostile prefix of 65 MiB must not reserve that much before a single byte of the body has arrived.
    let mut body = Vec::with_capacity((len as usize).min(1 << 20));
    match r.by_ref().take(len).read_to_end(&mut body) {
        Ok(n) if n as u64 == len => Ok(Some(body)),
        Ok(_) => Err(FrameError::Truncated),
        Err(e) => Err(FrameError::Io(e)),
    }
}

pub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame over 4 GiB"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut v = (body.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn a_frame_round_trips_and_a_clean_end_is_none() {
        let data = [framed(b"{\"a\":1}"), framed("h\u{e9}llo".as_bytes())].concat();
        let mut r = &data[..];
        assert_eq!(read_frame(&mut r, 100).unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(read_frame(&mut r, 100).unwrap().unwrap(), "h\u{e9}llo".as_bytes());
        assert!(read_frame(&mut r, 100).unwrap().is_none());
        let mut out = Vec::new();
        write_frame(&mut out, b"{}").unwrap();
        assert_eq!(out, framed(b"{}"));
    }

    #[test]
    fn a_cut_frame_is_truncated_and_an_oversize_prefix_is_refused_before_the_body() {
        let full = framed(b"abcdef");
        for cut in 1..full.len() {
            assert!(matches!(read_frame(&mut &full[..cut], 100), Err(FrameError::Truncated)), "cut at {cut}");
        }
        // The prefix says 101 bytes and only the prefix is there: TooLarge, not Truncated.
        assert!(matches!(read_frame(&mut &101u32.to_le_bytes()[..], 100), Err(FrameError::TooLarge(101))));
        assert!(matches!(read_frame(&mut &u32::MAX.to_le_bytes()[..], MAX_REQUEST), Err(FrameError::TooLarge(n)) if n == u32::MAX as u64));
        let exactly = framed(&[b'x'; 100]);
        assert_eq!(read_frame(&mut &exactly[..], 100).unwrap().unwrap().len(), 100);
    }
}
