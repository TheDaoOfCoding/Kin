//! Links in text: what counts as a URL for `isUrl`, which ones the PC may open, and opening them in the default browser.
//!
//! Text that arrives from the phone is untrusted. Only `http` and `https` links are ever handed to the shell, and the check
//! is made again at the moment of the click, whatever `isUrl` said.
use std::sync::Mutex;

use crate::Result;

/// Longest link the PC will hand to the shell. ShellExecute itself gives up at about 2,000 characters for some handlers,
/// and a link longer than this is not something a person would click.
pub const MAX_OPEN_BYTES: usize = 8192;

/// `isUrl` of protocol.md: the whole text, trimmed, is one absolute `http://` or `https://` URL with no whitespace.
/// (Control characters are refused as well: they have no place in a link, and a NUL would cut it short in the shell.)
pub fn is_http_url(text: &str) -> bool {
    let t = text.trim();
    if t.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let Some((scheme, rest)) = t.split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    // The authority runs up to the first `/`, `?` or `#`. It needs a host: `http://`, `http:///x` and `http://user@` have none.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    !host.is_empty() && !host.starts_with(':')
}

/// The link to open for a received `text`, or `None` if it must not be opened: the sender didn't set `isUrl`, the text isn't
/// one http(s) URL, or it's too long. The result is the trimmed text.
pub fn openable_url(text: &str, is_url: bool) -> Option<&str> {
    let t = text.trim();
    (is_url && is_http_url(t) && t.len() <= MAX_OPEN_BYTES).then_some(t)
}

/// Opens a link in the default browser. Behind a trait so tests can see what would be opened.
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
}

/// `ShellExecute` with the `open` verb. The URL is one argument for the shell, not a command line, so characters such as `&`
/// have no special meaning.
pub struct ShellOpener;

impl UrlOpener for ShellOpener {
    fn open(&self, url: &str) -> Result<()> {
        // The last line of defence: whatever calls this, nothing but an http(s) URL gets to the shell.
        if !is_http_url(url) || url.len() > MAX_OPEN_BYTES {
            return Err("refusing to open something that is not an http(s) link".into());
        }
        // ShellExecute may hand the link to shell extensions that need COM, and the toast click arrives on a thread that has not
        // initialised it. So it runs on a thread of its own that does (a single-threaded apartment, as the documentation asks).
        let url = url.trim().to_string();
        std::thread::Builder::new()
            .name("open-link".into())
            .spawn(move || shell_execute(&url))
            .map_err(|e| format!("could not start the thread that opens the link: {e}"))?
            .join()
            .map_err(|_| "the thread that opens the link panicked")?
    }
}

fn shell_execute(url: &str) -> Result<()> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    let url = HSTRING::from(url);
    // SAFETY: COM is initialised and uninitialised on this thread only if the initialisation worked, and both strings are
    // NUL-terminated and outlive the call.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = ShellExecuteW(None, w!("open"), &url, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
        if com {
            CoUninitialize();
        }
        // Values above 32 mean success.
        if result.0 as isize > 32 { Ok(()) } else { Err(format!("ShellExecute failed with code {}", result.0 as isize).into()) }
    }
}

/// For tests: remembers what it was asked to open.
#[derive(Default)]
pub struct RecordingOpener {
    pub opened: Mutex<Vec<String>>,
}

impl UrlOpener for RecordingOpener {
    fn open(&self, url: &str) -> Result<()> {
        self.opened.lock().unwrap_or_else(|p| p.into_inner()).push(url.to_string());
        Ok(())
    }
}

impl RecordingOpener {
    pub fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_one_absolute_http_or_https_url() {
        for ok in [
            "http://example.com",
            "https://example.com/a?b=c#d",
            "HTTPS://EXAMPLE.COM",
            "Http://example.com:8080/x",
            "  https://example.com/trimmed \r\n",
            "https://user:pw@example.com/",
            "https://192.168.1.5:47821/",
            "https://[::1]:8443/",
            "https://example.com/a%20b",
            "https://example.com/日本語",
            "http://localhost",
            "https://example.com/?q=a&b=c",
        ] {
            assert!(is_http_url(ok), "{ok:?}");
        }
    }

    #[test]
    fn refuses_every_other_scheme_and_anything_that_is_not_one_url() {
        for bad in [
            "",
            "example.com",
            "www.example.com",
            "//example.com",
            "ftp://example.com",
            "file:///C:/Windows/System32/calc.exe",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "ms-msdt:/id PCWDiagnostic",
            "mailto:a@b.c",
            "tel:+123",
            "kinrelay://pair?v=1",
            "http:/example.com",
            "http:example.com",
            "httpx://example.com",
            "xhttp://example.com",
            "ht tp://example.com",
            "http://",
            "https:///path",
            "http://?x",
            "http://#x",
            "http://user@",
            "http://:80/",
            "https://example.com/ and more",
            "see https://example.com",
            "https://example.com\nhttps://example.org",
            "https://example.com/\tx",
            "https://example.com/\u{0}",
            "https://example.com/\u{1b}[0m",
            "https://exa mple.com",
            "https://example.com/\u{a0}x",
            "\\\\server\\share",
        ] {
            assert!(!is_http_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_link_opens_only_when_the_sender_said_so_and_the_text_agrees() {
        assert_eq!(openable_url("https://example.com/x", true), Some("https://example.com/x"));
        assert_eq!(openable_url("  https://example.com/x\n", true), Some("https://example.com/x"));
        assert_eq!(openable_url("https://example.com/x", false), None, "isUrl is false");
        assert_eq!(openable_url("file:///C:/x.exe", true), None, "isUrl is true but the scheme is not http(s)");
        assert_eq!(openable_url("javascript:alert(1)", true), None);
        assert_eq!(openable_url("https://example.com/ rm -rf", true), None);
        let long = format!("https://example.com/{}", "a".repeat(MAX_OPEN_BYTES));
        assert!(is_http_url(&long));
        assert_eq!(openable_url(&long, true), None, "too long to hand to the shell");
    }

    #[test]
    fn the_shell_opener_refuses_other_schemes_before_it_reaches_the_shell() {
        // Nothing here can open anything: every one of these is refused first.
        for bad in ["file:///C:/Windows/System32/calc.exe", "calc.exe", "ms-msdt:/id x", "javascript:1", "", "https://"] {
            assert!(ShellOpener.open(bad).is_err(), "{bad:?}");
        }
    }
}
