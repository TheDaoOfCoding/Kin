//! The Mark of the Web on every received file (windows.md, "Received files").
//!
//! A file that comes from outside the PC gets an NTFS alternate data stream called `Zone.Identifier` that says where it came from
//! (`ZoneId=3` is the Internet zone). Windows reads it when the file is opened: SmartScreen checks a program before it runs, Office
//! opens a document in Protected View (macros and active content are off until the user allows them), and Explorer shows "this file
//! came from another computer" in its properties. Browsers and mail programs write the same stream. A file from the phone is as
//! untrusted as a download (any app on the phone can send a file, R13), so it carries the mark too.
//!
//! The stream is written on the `.part` file just before the final move, so the file has the mark from the moment it has its final
//! name (the stream moves with the file). A volume that has no streams (FAT, exFAT, some network shares) can't carry it: the write fails,
//! the file is received all the same, and the log says the mark could not be written (one line, no name).
use std::ffi::OsString;
use std::io;
use std::path::Path;

/// The text of the stream. `ZoneId=3` is "Internet"; `HostUrl=about:internet` is what Windows writes when it doesn't know the address.
pub const ZONE_IDENTIFIER: &str = "[ZoneTransfer]\r\nZoneId=3\r\nHostUrl=about:internet\r\n";

fn stream_path(file: &Path) -> OsString {
    let mut name = file.as_os_str().to_os_string();
    name.push(":Zone.Identifier");
    name
}

/// Writes the Mark of the Web onto `file` (which has to exist).
pub fn write(file: &Path) -> io::Result<()> {
    std::fs::write(stream_path(file), ZONE_IDENTIFIER)
}

/// The text of the `Zone.Identifier` stream of `file`, or `None` if it has none (or the volume can't hold one).
pub fn read(file: &Path) -> Option<String> {
    std::fs::read_to_string(stream_path(file)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mark_says_internet_zone_and_can_be_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("photo.jpg");
        std::fs::write(&file, "pixels").unwrap();
        assert_eq!(read(&file), None, "no mark before");
        write(&file).unwrap();
        let text = read(&file).expect("the stream is there (the temp folder is on NTFS)");
        assert!(text.starts_with("[ZoneTransfer]\r\n") && text.contains("\r\nZoneId=3\r\n"), "{text:?}");
        // The file's own bytes are untouched, and the stream isn't a file of the folder.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "pixels");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn the_mark_goes_with_the_file_when_it_is_moved() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.bin.part");
        std::fs::write(&part, "x").unwrap();
        write(&part).unwrap();
        let done = dir.path().join("a.bin");
        std::fs::rename(&part, &done).unwrap();
        assert!(read(&done).is_some_and(|t| t.contains("ZoneId=3")));
    }
}
