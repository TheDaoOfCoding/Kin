//! Folders and disks: where Downloads is, and how much room a disk has.
use std::path::{Path, PathBuf};

/// The user's Downloads folder. It is asked from Windows (the folder can be moved), and `%USERPROFILE%\Downloads` is the fallback.
pub fn downloads_dir() -> PathBuf {
    #[cfg(windows)]
    {
        use windows::Win32::System::Com::CoTaskMemFree;
        use windows::Win32::UI::Shell::{FOLDERID_Downloads, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
        // SAFETY: the string that comes back is copied and then freed with `CoTaskMemFree`, as the documentation says.
        unsafe {
            if let Ok(p) = SHGetKnownFolderPath(&FOLDERID_Downloads, KF_FLAG_DEFAULT, None) {
                let text = p.to_string();
                CoTaskMemFree(Some(p.0 as *const _));
                if let Ok(text) = text.map(PathBuf::from) {
                    return text;
                }
            }
        }
    }
    std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("Downloads"), |p| PathBuf::from(p).join("Downloads"))
}

/// A path of at least this many characters is written with the `\\?\` prefix when it goes to a Win32 call that has no long-path support
/// of its own (the rest of `std::fs` adds the prefix by itself). `MAX_PATH` is 260 including the end of string, and a folder has to leave
/// room for a name of 12 characters, so 240 keeps a margin.
const LONG_PATH_FROM: usize = 240;

/// The form of `path` that Win32 file calls accept when it is longer than `MAX_PATH` (`\\?\C:\...`, or `\\?\UNC\server\share\...`). A
/// short path, a path that already has the prefix and one that can't be made absolute are returned as they are. Use it for the calls
/// that take a path straight from the `windows` crate (`MoveFileExW`, `GetDiskFreeSpaceExW`, the imaging calls): `std::fs` does it
/// by itself.
pub fn long_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.as_os_str().to_string_lossy();
        if text.encode_utf16().count() < LONG_PATH_FROM || text.starts_with(r"\\?\") || text.starts_with(r"\\.\") {
            return path.to_path_buf();
        }
        // `\\?\` turns off the cleaning that Windows does (`/`, `.`, `..`), so the path is made absolute and clean first.
        let Ok(absolute) = std::path::absolute(path) else {
            return path.to_path_buf();
        };
        let text = absolute.to_string_lossy().replace('/', r"\");
        match text.strip_prefix(r"\\") {
            Some(unc) => PathBuf::from(format!(r"\\?\UNC\{unc}")),
            None => PathBuf::from(format!(r"\\?\{text}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = LONG_PATH_FROM;
        path.to_path_buf()
    }
}

/// The free space on the disk that holds `dir` (or the nearest folder above it that exists): what is available to this user, so
/// quotas count. `None` if Windows can't say.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    let mut existing = dir;
    while !existing.exists() {
        existing = existing.parent()?;
    }
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        use windows::core::HSTRING;
        let mut free = 0u64;
        // SAFETY: the path is a NUL-terminated string, and `free` is a valid place for the answer.
        unsafe { GetDiskFreeSpaceExW(&HSTRING::from(long_path(existing).as_os_str()), Some(&mut free), None, None).ok()? };
        Some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = existing;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temp_folder_has_free_space_and_a_new_subfolder_is_asked_about_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_bytes(dir.path()).expect("the temp disk answers") > 0);
        assert!(free_bytes(&dir.path().join("not").join("there").join("yet")).expect("the nearest folder that exists answers") > 0);
    }

    #[cfg(windows)]
    #[test]
    fn a_long_path_gets_the_verbatim_prefix_and_a_short_one_does_not() {
        assert_eq!(long_path(Path::new(r"C:\a\b.txt")), Path::new(r"C:\a\b.txt"));
        let deep = format!(r"C:\{}\x.txt", vec!["d".repeat(60); 5].join(r"\"));
        assert!(deep.len() > LONG_PATH_FROM);
        assert_eq!(long_path(Path::new(&deep)), Path::new(&format!(r"\\?\{deep}")));
        let unc = format!(r"\\server\share\{}\x.txt", vec!["d".repeat(60); 5].join(r"\"));
        assert_eq!(long_path(Path::new(&unc)), Path::new(&format!(r"\\?\UNC\{}", &unc[2..])));
        let done = format!(r"\\?\{deep}");
        assert_eq!(long_path(Path::new(&done)), Path::new(&done), "already prefixed");
        let dotted = format!(r"C:\{}\..\y\x.txt", vec!["d".repeat(60); 4].join(r"\"));
        assert!(!long_path(Path::new(&dotted)).to_string_lossy().contains(".."), "`..` is resolved before the prefix goes on");
    }

    #[test]
    fn downloads_is_a_folder_path() {
        let d = downloads_dir();
        assert!(d.is_absolute(), "{}", d.display());
        assert!(d.file_name().is_some());
    }
}
