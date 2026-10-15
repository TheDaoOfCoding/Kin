//! Links in text: what counts as a URL for `isUrl`, which ones the PC may open, and opening them in the default browser. Also
//! opening a received file or its folder from a toast, which is the same kind of risk (something the phone sent, handed to the
//! shell) and is handled the same careful way.
//!
//! Text that arrives from the phone is untrusted. Only `http` and `https` links are ever handed to the shell, and the check
//! is made again at the moment of the click, whatever `isUrl` said. A received file is opened from a toast only if its type is on an
//! **allow-list** of safe ones (pictures, audio, video, PDF, text, macro-free Office files); anything else is shown in its folder.
use std::path::Path;
use std::sync::Mutex;

use crate::Result;

/// The longest link: 8 KiB of UTF-8 after trimming (protocol.md, "Text messages").
pub const MAX_URL_BYTES: usize = 8192;

/// The Unicode `White_Space` set, listed in protocol.md so no platform's own `isWhitespace()` decides: U+0009 to U+000D, U+0020,
/// U+0085, U+00A0, U+1680, U+2000 to U+200A, U+2028, U+2029, U+202F, U+205F and U+3000. (Rust's `char::is_whitespace` is the same set,
/// but the list is written here so a change of the standard library can't change the protocol.)
pub fn is_white_space(c: char) -> bool {
    matches!(c, '\u{9}'..='\u{d}' | ' ' | '\u{85}' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

/// Control characters: U+0000 to U+001F and U+007F to U+009F.
fn is_control(c: char) -> bool {
    c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c)
}

/// Removes leading and trailing `White_Space`.
pub fn trim(text: &str) -> &str {
    text.trim_matches(is_white_space)
}

/// The rule of protocol.md ("Text messages", "Links: `isUrl` and openable URLs"): after trimming, the scheme is `http` or `https`
/// followed by `://`, there is no whitespace or control character anywhere, the host is not empty and not only dots (or is a
/// non-empty `[IPv6]` literal), a port (if there is a `:`) is 1 to 65535 in ASCII digits, and the whole text is at most 8 KiB.
pub fn is_openable_url(text: &str) -> bool {
    let t = trim(text);
    if t.len() > MAX_URL_BYTES || t.chars().any(|c| is_white_space(c) || is_control(c)) {
        return false;
    }
    let Some((scheme, rest)) = t.split_once("://") else {
        return false;
    };
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return false;
    }
    // The authority runs up to the first `/`, `?` or `#`. Everything up to the last `@` is user information and is ignored.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host_part = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let port = if let Some(bracketed) = host_part.strip_prefix('[') {
        let Some((inside, after)) = bracketed.split_once(']') else {
            return false;
        };
        if inside.is_empty() || inside.contains('[') {
            return false;
        }
        match after {
            "" => None,
            _ => match after.strip_prefix(':') {
                Some(port) => Some(port),
                None => return false,
            },
        }
    } else {
        let (host, port) = match host_part.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (host_part, None),
        };
        if host.is_empty() || host.chars().all(|c| c == '.') {
            return false;
        }
        port
    };
    port.is_none_or(valid_port)
}

/// ASCII digits only, value 1 to 65535, leading zeros allowed.
fn valid_port(port: &str) -> bool {
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let significant = port.trim_start_matches('0');
    !significant.is_empty() && significant.len() <= 5 && significant.parse::<u32>().is_ok_and(|n| n <= 65535)
}

/// The `isUrl` a sender writes. (Kept under its old name: the callers say what they mean.)
pub fn is_http_url(text: &str) -> bool {
    is_openable_url(text)
}

/// The link to open for a received `text`, or `None` if it must not be opened: the sender didn't set `isUrl`, or the text isn't an
/// openable URL by the rule above. The result is the trimmed text.
pub fn openable_url(text: &str, is_url: bool) -> Option<&str> {
    (is_url && is_openable_url(text)).then(|| trim(text))
}

/// The file types a toast may open with their default program: **an allow-list**. Everything else is shown in its folder, selected,
/// and the user opens it on purpose. (A deny-list can't work: any program a user has installed registers its own types, so `.sh` opens
/// in Git Bash, `.pyz` runs under Python, and the list of what Windows can run is never complete. Any app on the phone can send a
/// file, so a toast click must never be able to start a program.)
///
/// Pictures, audio and video, PDF, plain text (`csv` and `md` open as text), and the Office and OpenDocument formats **without
/// macros** (`docx`, `xlsx`, `pptx`, `odt`, `ods`, `odp`; the macro-enabled `docm`, `xlsm`, ... are not in it). Lower case, no dot.
pub const OPENABLE_EXTENSIONS: &[&str] = &[
    // pictures
    "jpg", "jpeg", "png", "gif", "webp", "heic", "heif", "bmp", "tiff", // audio and video
    "mp3", "m4a", "aac", "flac", "wav", "ogg", "opus", "mp4", "mov", "mkv", "webm", "avi", "3gp", // documents
    "pdf", "txt", "csv", "md", "docx", "xlsx", "pptx", "odt", "ods", "odp",
];

/// Whether a received file may be opened with its default program: its **final** extension (ASCII, case-insensitive) is in
/// [`OPENABLE_EXTENSIONS`]. Only the last extension counts, so `evil.pdf.exe` is not openable and `evil.exe.pdf` is a PDF. Trailing dots
/// and spaces are ignored (Windows ignores them), and a name with a stream (`:`) or a character a name can't have is never
/// openable. A name with no extension, or one that isn't valid text, isn't either.
pub fn may_open_file(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.trim_end_matches(['.', ' ']);
    if name.contains([':', '/', '\\', '<', '>', '"', '|', '?', '*']) || name.chars().any(char::is_control) {
        return false;
    }
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    ext.bytes().all(|b| b.is_ascii_alphanumeric()) && OPENABLE_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Opens things for the user. Behind a trait so tests can see what would be opened.
pub trait UrlOpener: Send + Sync {
    fn open(&self, url: &str) -> Result<()>;
    /// Opens a file with its default program. Never called for one that isn't on the allow-list ([`may_open_file`]).
    fn open_path(&self, path: &Path) -> Result<()>;
    /// Opens a folder in Explorer with the file selected.
    fn show_in_folder(&self, path: &Path) -> Result<()>;
    /// Opens a folder in Explorer.
    fn open_folder(&self, folder: &Path) -> Result<()>;
}

/// `ShellExecute` with the `open` verb. The URL is one argument for the shell, not a command line, so characters such as `&`
/// have no special meaning.
pub struct ShellOpener;

impl UrlOpener for ShellOpener {
    fn open(&self, url: &str) -> Result<()> {
        // The last line of defence: whatever calls this, nothing but an http(s) URL gets to the shell.
        if !is_openable_url(url) {
            return Err("refusing to open something that is not an http(s) link".into());
        }
        shell_execute_on_thread(trim(url).to_string())
    }

    fn open_path(&self, path: &Path) -> Result<()> {
        if !may_open_file(path) {
            return Err("refusing to open a file whose type isn't on the allow-list".into());
        }
        shell_execute_on_thread(crate::paths::long_path(path).to_string_lossy().into_owned())
    }

    fn show_in_folder(&self, path: &Path) -> Result<()> {
        use std::os::windows::process::CommandExt;
        // `/select,"<path>"` is one argument with its own quoting, which `Command::arg` would quote a second time.
        std::process::Command::new("explorer.exe")
            .raw_arg(format!("/select,\"{}\"", crate::paths::long_path(path).display()))
            .spawn()
            .map(|_| ())
            .map_err(Into::into)
    }

    fn open_folder(&self, folder: &Path) -> Result<()> {
        std::process::Command::new("explorer.exe").arg(folder).spawn().map(|_| ()).map_err(Into::into)
    }
}

/// ShellExecute may hand the target to shell extensions that need COM, and the toast click arrives on a thread that has not
/// initialised it. So it runs on a thread of its own that does (a single-threaded apartment, as the documentation asks).
fn shell_execute_on_thread(target: String) -> Result<()> {
    std::thread::Builder::new()
        .name("open-link".into())
        .spawn(move || shell_execute(&target))
        .map_err(|e| format!("could not start the thread that opens the link: {e}"))?
        .join()
        .map_err(|_| "the thread that opens the link panicked")?
}

fn shell_execute(target: &str) -> Result<()> {
    use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    let target = HSTRING::from(target);
    // SAFETY: COM is initialised and uninitialised on this thread only if the initialisation worked, and both strings are
    // NUL-terminated and outlive the call.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = ShellExecuteW(None, w!("open"), &target, PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
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
    pub files: Mutex<Vec<std::path::PathBuf>>,
    pub shown: Mutex<Vec<std::path::PathBuf>>,
    pub folders: Mutex<Vec<std::path::PathBuf>>,
}

impl UrlOpener for RecordingOpener {
    fn open(&self, url: &str) -> Result<()> {
        self.opened.lock().unwrap_or_else(|p| p.into_inner()).push(url.to_string());
        Ok(())
    }

    fn open_path(&self, path: &Path) -> Result<()> {
        self.files.lock().unwrap_or_else(|p| p.into_inner()).push(path.to_path_buf());
        Ok(())
    }

    fn show_in_folder(&self, path: &Path) -> Result<()> {
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).push(path.to_path_buf());
        Ok(())
    }

    fn open_folder(&self, folder: &Path) -> Result<()> {
        self.folders.lock().unwrap_or_else(|p| p.into_inner()).push(folder.to_path_buf());
        Ok(())
    }
}

impl RecordingOpener {
    pub fn opened(&self) -> Vec<String> {
        self.opened.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Files opened with their default program.
    pub fn opened_files(&self) -> Vec<std::path::PathBuf> {
        self.files.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Files shown selected in their folder.
    pub fn shown_in_folder(&self) -> Vec<std::path::PathBuf> {
        self.shown.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Folders opened.
    pub fn opened_folders(&self) -> Vec<std::path::PathBuf> {
        self.folders.lock().unwrap_or_else(|p| p.into_inner()).clone()
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
            assert!(is_openable_url(ok), "{ok:?}");
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
            "https://example.com:",
            "https://example.com:0",
            "https://example.com:65536",
            "https://./",
            "http://[]/",
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
            assert!(!is_openable_url(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_size_limit_is_8_kib_of_the_trimmed_text() {
        let url = |n: usize| format!("https://e.com/{}", "a".repeat(n));
        assert!(is_openable_url(&url(MAX_URL_BYTES - 14)));
        assert!(!is_openable_url(&url(MAX_URL_BYTES - 13)));
        assert!(is_openable_url(&format!("{}   \n", url(MAX_URL_BYTES - 14))), "trailing whitespace is trimmed first");
    }

    #[test]
    fn a_link_opens_only_when_the_sender_said_so_and_the_text_agrees() {
        assert_eq!(openable_url("https://example.com/x", true), Some("https://example.com/x"));
        assert_eq!(openable_url("  https://example.com/x\n", true), Some("https://example.com/x"));
        assert_eq!(openable_url("https://example.com/x", false), None, "isUrl is false");
        assert_eq!(openable_url("file:///C:/x.exe", true), None, "isUrl is true but the scheme is not http(s)");
        assert_eq!(openable_url("javascript:alert(1)", true), None);
        assert_eq!(openable_url("https://example.com/ rm -rf", true), None);
        let long = format!("https://example.com/{}", "a".repeat(MAX_URL_BYTES));
        assert_eq!(openable_url(&long, true), None, "too long");
    }

    #[test]
    fn the_shell_opener_refuses_other_schemes_before_it_reaches_the_shell() {
        // Nothing here can open anything: every one of these is refused first.
        for bad in ["file:///C:/Windows/System32/calc.exe", "calc.exe", "ms-msdt:/id x", "javascript:1", "", "https://"] {
            assert!(ShellOpener.open(bad).is_err(), "{bad:?}");
        }
        for risky in
            ["C:\\x\\setup.exe", "C:\\x\\run.bat", "C:\\x\\a.lnk", "C:\\x\\run.sh", "C:\\x\\a.pyz", "C:\\x\\unknown.xyz", "C:\\x\\noext"]
        {
            assert!(ShellOpener.open_path(Path::new(risky)).is_err(), "{risky}");
        }
    }

    #[test]
    fn only_the_allow_listed_types_may_be_opened_by_their_last_extension() {
        for safe in [
            "photo.jpg",
            "photo.JPG",
            "a.jpeg",
            "a.png",
            "a.gif",
            "a.webp",
            "a.heic",
            "a.heif",
            "a.bmp",
            "a.tiff",
            "song.mp3",
            "a.m4a",
            "a.aac",
            "a.flac",
            "a.wav",
            "a.ogg",
            "a.opus",
            "clip.mp4",
            "a.mov",
            "a.mkv",
            "a.webm",
            "a.avi",
            "a.3gp",
            "report.pdf",
            "REPORT.PDF",
            "notes.txt",
            "a.csv",
            "a.md",
            "a.docx",
            "a.xlsx",
            "a.pptx",
            "a.odt",
            "a.ods",
            "a.odp",
            "evil.exe.pdf",
            "two.dots.in.the.name.txt",
            ".txt",
            "a.pdf.",
            "a.pdf ",
            "a b c.png",
        ] {
            assert!(may_open_file(Path::new(safe)), "{safe}");
        }
        assert_eq!(OPENABLE_EXTENSIONS.len(), 9 + 13 + 10, "the allow-list is exactly what the review asked for");
    }

    #[test]
    fn everything_else_is_shown_in_its_folder_not_opened() {
        for unsafe_name in [
            // programs and scripts that Windows runs
            "setup.exe",
            "A.EXE",
            "run.bat",
            "x.cmd",
            "s.ps1",
            "a.vbs",
            "a.js",
            "a.jse",
            "a.wsf",
            "a.hta",
            "a.msi",
            "a.msp",
            "a.dll",
            "a.scr",
            "a.com",
            "a.pif",
            "a.cpl",
            "a.msc",
            "a.reg",
            "a.inf",
            "a.jar",
            "a.application",
            "a.gadget",
            "a.appx",
            "a.msix",
            // shortcuts and things that open something else
            "a.lnk",
            "a.url",
            "a.website",
            "a.scf",
            "a.library-ms",
            "a.search-ms",
            "a.settingcontent-ms",
            "a.diagcab",
            "a.appinstaller",
            "a.rdp",
            "a.theme",
            // what the old deny-list missed: other programs register these types on this PC and on any developer's
            "a.sh",
            "a.pyz",
            "a.py",
            "a.pyw",
            "a.pyc",
            "a.wsb",
            "a.ahk",
            "a.vsto",
            "a.jnlp",
            "a.bash",
            "a.zsh",
            "a.pl",
            "a.rb",
            "a.php",
            "a.ps1xml",
            // macros and active content
            "a.docm",
            "a.xlsm",
            "a.pptm",
            "a.xll",
            "a.iqy",
            "a.slk",
            "a.chm",
            "a.hlp",
            "a.svg",
            "a.html",
            "a.htm",
            "a.xml",
            "a.json",
            "a.zip",
            "a.apk",
            // double extensions: only the last one counts
            "evil.pdf.exe",
            "photo.jpg.lnk",
            "doc.txt.bat",
            "x.pdf.sh",
            // no extension, odd names
            "noextension",
            "exe",
            "",
            "a.",
            "...",
            "a.pdf:stream.exe",
            "a.txt:Zone.Identifier",
            "a.pdf\u{0}.exe",
            "a.p df",
            "a.\u{ff50}df",
            "a.pdf*",
        ] {
            assert!(!may_open_file(Path::new(unsafe_name)), "{unsafe_name:?}");
        }
        // Windows ignores trailing dots and spaces, so they can't hide a program.
        for hidden in ["a.exe.", "a.exe ", "a.exe . ", "a.bat..."] {
            assert!(!may_open_file(Path::new(hidden)), "{hidden:?}");
        }
        assert!(!may_open_file(Path::new(".exe")), "a file named just .exe is still an executable to Windows");
    }
}
