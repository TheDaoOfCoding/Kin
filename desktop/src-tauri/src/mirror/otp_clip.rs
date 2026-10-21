//! "Copy <code>" of a toast and the 60-second clear (design 5.7).
//!
//! * **Copy** needs no round trip to the phone. The code goes to the clipboard with the **sensitive** formats (`write_text_sensitive`:
//!   `ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory` = 0, `CanUploadToCloudClipboard` = 0), so Windows' clipboard history and cloud
//!   clipboard don't keep it **and Kin's own automatic clipboard sync doesn't send it back to the phone**. The clipboard's change counter after the write
//!   and the code are remembered, in memory only.
//! * **After 60 seconds** the clipboard is emptied **only if** its counter is still that number **and** its text is still the code (the check and the
//!   clearing are one step, `Clipboard::clear_if_text_unchanged`): anything the user copied in the meantime is never touched and never read. A second Copy
//!   starts the 60 seconds again. When Kin quits the same check runs once at once.
//!
//! The code is never logged: a log line says that a code was copied or cleared, and nothing else.
use crate::clipboard::Clipboard;

/// How long a copied code stays on the clipboard.
pub const CLEAR_AFTER_MS: u64 = 60_000;

/// What the next clear is about.
struct Pending {
    code: String,
    sequence: Option<u32>,
    due_ms: u64,
}

#[derive(Default)]
pub struct OtpClip {
    pending: Option<Pending>,
}

/// What a clear did, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cleared {
    /// Nothing was waiting, or it is not time yet.
    Nothing,
    /// The clipboard held the code and nothing else had happened: it is empty now.
    Emptied,
    /// Something else is on the clipboard (or the counter moved): left alone, not read.
    LeftAlone,
    Failed,
}

impl OtpClip {
    /// Puts `code` on the clipboard as sensitive and arms the clear for `now_ms + 60 s`. Returns when the clear is due.
    pub fn copy(&mut self, clipboard: &dyn Clipboard, code: &str, now_ms: u64) -> Result<u64, String> {
        clipboard.write_text_sensitive(code).map_err(|e| e.to_string())?;
        let due_ms = now_ms + CLEAR_AFTER_MS;
        self.pending = Some(Pending { code: code.to_string(), sequence: clipboard.sequence_number(), due_ms });
        Ok(due_ms)
    }

    /// When the armed clear is due, if one is.
    pub fn due_at(&self) -> Option<u64> {
        self.pending.as_ref().map(|p| p.due_ms)
    }

    /// The clear, if it is due: one look at the clipboard, then it is over (whatever the answer was).
    pub fn clear_if_due(&mut self, clipboard: &dyn Clipboard, now_ms: u64) -> Cleared {
        if self.pending.as_ref().is_none_or(|p| now_ms < p.due_ms) {
            return Cleared::Nothing;
        }
        self.clear_now(clipboard)
    }

    /// The same check at once (when Kin quits).
    pub fn clear_now(&mut self, clipboard: &dyn Clipboard) -> Cleared {
        let Some(p) = self.pending.take() else {
            return Cleared::Nothing;
        };
        match clipboard.clear_if_text_unchanged(&p.code, p.sequence) {
            Ok(true) => Cleared::Emptied,
            Ok(false) => Cleared::LeftAlone,
            Err(_) => Cleared::Failed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{CAN_INCLUDE_IN_HISTORY, CAN_UPLOAD_TO_CLOUD, EXCLUDE_FROM_MONITOR, MemoryClipboard};

    #[test]
    fn a_copied_code_carries_the_sensitive_formats_so_the_sync_and_the_history_leave_it_alone() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        otp.copy(&clip, "482913", 1000).unwrap();
        assert_eq!(clip.text().as_deref(), Some("482913"));
        assert_eq!(clip.app_sensitive_writes(), ["482913"]);
        assert!(clip.app_writes().is_empty(), "not an ordinary write");
        let formats = clip.read().unwrap().formats;
        assert!(formats.iter().any(|f| f.name == EXCLUDE_FROM_MONITOR));
        assert!(formats.iter().any(|f| f.name == CAN_INCLUDE_IN_HISTORY && f.dword == Some(0)));
        assert!(formats.iter().any(|f| f.name == CAN_UPLOAD_TO_CLOUD && f.dword == Some(0)));
        assert_eq!(crate::clipboard::exclusion(&formats), Some(crate::clipboard::Exclusion::ExcludeFromMonitor), "Kin's own sync skips it");
    }

    #[test]
    fn after_60_seconds_an_unchanged_clipboard_is_emptied() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        assert_eq!(otp.copy(&clip, "482913", 5000), Ok(65_000));
        assert_eq!(otp.clear_if_due(&clip, 64_999), Cleared::Nothing, "not yet");
        assert_eq!(clip.text().as_deref(), Some("482913"));
        assert_eq!(otp.clear_if_due(&clip, 65_000), Cleared::Emptied);
        assert_eq!(clip.text(), None);
        assert_eq!(otp.clear_if_due(&clip, 70_000), Cleared::Nothing, "it is over");
    }

    #[test]
    fn a_clipboard_the_user_changed_is_left_alone() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        otp.copy(&clip, "482913", 0).unwrap();
        clip.user_copy("something else", vec![]);
        assert_eq!(otp.clear_if_due(&clip, 60_000), Cleared::LeftAlone);
        assert_eq!(clip.text().as_deref(), Some("something else"));
    }

    #[test]
    fn the_same_text_copied_again_by_someone_else_is_left_alone_too_because_the_counter_moved() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        otp.copy(&clip, "482913", 0).unwrap();
        clip.user_copy("482913", vec![]);
        assert_eq!(otp.clear_if_due(&clip, 60_000), Cleared::LeftAlone);
        assert_eq!(clip.text().as_deref(), Some("482913"));
    }

    #[test]
    fn a_second_copy_starts_the_60_seconds_again() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        otp.copy(&clip, "111111", 0).unwrap();
        otp.copy(&clip, "222222", 30_000).unwrap();
        assert_eq!(otp.due_at(), Some(90_000));
        assert_eq!(otp.clear_if_due(&clip, 60_000), Cleared::Nothing, "the first copy's time is over, the second's is not");
        assert_eq!(otp.clear_if_due(&clip, 90_000), Cleared::Emptied);
        assert_eq!(clip.text(), None);
    }

    #[test]
    fn quitting_runs_the_check_at_once() {
        let clip = MemoryClipboard::default();
        let mut otp = OtpClip::default();
        otp.copy(&clip, "482913", 0).unwrap();
        assert_eq!(otp.clear_now(&clip), Cleared::Emptied);
        assert_eq!(otp.clear_now(&clip), Cleared::Nothing);
    }
}
