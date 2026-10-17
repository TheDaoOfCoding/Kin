//! The native messaging framing, used on stdio and on the pipe to Kin: a 32-bit little-endian length, then that many bytes of UTF-8 JSON.

use std::io::{self, Read, Write};

/// The most the extension may send to the host (Chrome's limit; docs/extension.md "Framing").
pub const MAX_REQUEST: usize = 64 * 1024 * 1024;
/// The most the host may send to the browser: a bigger message makes the browser drop the connection.
pub const MAX_RESPONSE: usize = 1024 * 1024;
/// A message on the pipe to Kin, either way: 64 MiB request plus the fields the host adds, or 64 MiB + 1 MiB.
pub const MAX_PIPE_MESSAGE: usize = MAX_REQUEST + MAX_RESPONSE;

#[derive(Debug)]
pub enum FrameError {
    /// The stream ended inside a header or a body.
    Truncated,
    /// The length prefix is over the limit (the body is NOT read).
    TooLarge(u64),
    Io(io::Error),
}

/// Reads one frame. `Ok(None)` is a clean end of stream at a frame boundary. `spare` is extra capacity to reserve in the returned buffer,
/// so the caller can append to it without a copy of a body that can be 64 MiB.
pub fn read_frame<R: Read>(r: &mut R, max: usize, spare: usize) -> Result<Option<Vec<u8>>, FrameError> {
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
    let len = len as usize;
    let mut body = Vec::with_capacity(len + spare);
    match r.by_ref().take(len as u64).read_to_end(&mut body) {
        Ok(n) if n == len => Ok(Some(body)),
        Ok(_) => Err(FrameError::Truncated),
        Err(e) => Err(FrameError::Io(e)),
    }
}

/// Writes one frame.
pub fn write_frame<W: Write>(w: &mut W, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "frame over 4 GiB"))?;
    w.write_all(&len.to_le_bytes())?;
    w.write_all(body)?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that hands out at most `step` bytes per call (the way a pipe does).
    struct Trickle<'a> {
        data: &'a [u8],
        step: usize,
    }
    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.step.min(buf.len()).min(self.data.len());
            buf[..n].copy_from_slice(&self.data[..n]);
            self.data = &self.data[n..];
            Ok(n)
        }
    }

    fn framed(body: &[u8]) -> Vec<u8> {
        let mut v = (body.len() as u32).to_le_bytes().to_vec();
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn reads_whole_and_partial_frames() {
        let data = [framed(b"{\"a\":1}"), framed(b"{}")].concat();
        for step in [1, 2, 3, 5, 1000] {
            let mut r = Trickle { data: &data, step };
            assert_eq!(read_frame(&mut r, 100, 0).unwrap().unwrap(), b"{\"a\":1}", "step {step}");
            assert_eq!(read_frame(&mut r, 100, 0).unwrap().unwrap(), b"{}", "step {step}");
            assert!(read_frame(&mut r, 100, 0).unwrap().is_none(), "clean EOF at a boundary, step {step}");
        }
    }

    #[test]
    fn zero_length_frame_is_a_frame() {
        let mut r = Trickle { data: &framed(b""), step: 2 };
        assert_eq!(read_frame(&mut r, 100, 0).unwrap().unwrap(), b"");
    }

    #[test]
    fn eof_inside_the_header_or_the_body_is_truncated() {
        for cut in 1..4 {
            let mut r = Trickle { data: &framed(b"abcdef")[..cut], step: 1 };
            assert!(matches!(read_frame(&mut r, 100, 0), Err(FrameError::Truncated)), "cut at {cut}");
        }
        let full = framed(b"abcdef");
        let mut r = Trickle { data: &full[..full.len() - 1], step: 3 };
        assert!(matches!(read_frame(&mut r, 100, 0), Err(FrameError::Truncated)));
    }

    #[test]
    fn oversize_prefix_is_refused_without_reading_the_body() {
        // The prefix says 101 bytes and only the prefix is there: it must fail with TooLarge, not Truncated.
        let mut r = Trickle { data: &101u32.to_le_bytes(), step: 4 };
        assert!(matches!(read_frame(&mut r, 100, 0), Err(FrameError::TooLarge(101))));
        // Exactly the limit is fine.
        let body = vec![b'x'; 100];
        let mut r = Trickle { data: &framed(&body), step: 7 };
        assert_eq!(read_frame(&mut r, 100, 0).unwrap().unwrap().len(), 100);
        // A prefix of 4 GiB - 1 is just another too-large one.
        let mut r = Trickle { data: &u32::MAX.to_le_bytes(), step: 4 };
        assert!(matches!(read_frame(&mut r, MAX_REQUEST, 0), Err(FrameError::TooLarge(n)) if n == u32::MAX as u64));
    }

    #[test]
    fn spare_capacity_is_reserved() {
        let mut r = Trickle { data: &framed(b"{}"), step: 100 };
        let v = read_frame(&mut r, 100, 64).unwrap().unwrap();
        assert!(v.capacity() >= v.len() + 64);
    }

    #[test]
    fn write_then_read_roundtrip() {
        let mut out = Vec::new();
        write_frame(&mut out, "héllo \u{1F600}".as_bytes()).unwrap();
        assert_eq!(&out[..4], &(("héllo \u{1F600}".len()) as u32).to_le_bytes());
        let got = read_frame(&mut &out[..], 100, 0).unwrap().unwrap();
        assert_eq!(got, "héllo \u{1F600}".as_bytes());
    }
}
