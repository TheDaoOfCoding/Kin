//! File names of received files (protocol.md, "File names"): cleaning, the collision suffix and the length limit, and the
//! create-new rule (a file is never overwritten, "already exists" is a collision and never an error).
//!
//! Everything here is a pure function except [`create_part`], [`move_part`] and [`existing_names`], which touch a folder. The
//! lengths are UTF-8 **bytes** and a cut only ever falls on a code point boundary. `file-names.json` has the vectors.
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

pub const MAX_NAME_BYTES: usize = 200;
pub const MAX_EXT_BYTES: usize = 16;
/// How many candidates are tried (the plain name and ` (1)` to ` (99)`) before the create is given up as a failure, which is
/// `internal` (protocol.md, "File names"). A folder with a hundred files of one name is not something a person has, and a limit
/// keeps a hostile phone from making the PC probe the folder forever.
pub const MAX_CANDIDATES: u32 = 100;

/// `Bidi_Control`: U+061C, U+200E, U+200F, U+202A to U+202E and U+2066 to U+2069.
pub fn is_bidi_control(c: char) -> bool {
    matches!(c, '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Whether the part before the first dot (trailing spaces removed) is a reserved Windows device name, ignoring ASCII case:
/// `CON`, `PRN`, `AUX`, `NUL`, `COM0` to `COM9`, `LPT0` to `LPT9`, and `COM` or `LPT` followed by `¹`, `²` or `³`.
pub fn is_reserved(name: &str) -> bool {
    let before_dot = name.split('.').next().unwrap_or("").trim_end_matches(' ');
    let upper = before_dot.to_ascii_uppercase();
    if matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL") {
        return true;
    }
    let mut chars = upper.chars();
    let (a, b, c, d) = (chars.next(), chars.next(), chars.next(), chars.next());
    let device = matches!((a, b, c), (Some('C'), Some('O'), Some('M')) | (Some('L'), Some('P'), Some('T')));
    device && chars.next().is_none() && d.is_some_and(|d| d.is_ascii_digit() || matches!(d, '\u{b9}' | '\u{b2}' | '\u{b3}'))
}

/// Steps 1 to 5 of the spec: the part after the last `/` or `\`, forbidden characters, control characters and bidi controls
/// replaced by `_`, trailing dots and spaces removed, an empty name becomes `file`, a reserved name gets a `_` in front.
pub fn clean(input: &str) -> String {
    let base = input.rsplit(['/', '\\']).next().unwrap_or("");
    let replaced: String = base
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control() || is_bidi_control(c) { '_' } else { c }
        })
        .collect();
    let trimmed = replaced.trim_end_matches(['.', ' ']);
    let name = if trimmed.is_empty() { "file".to_string() } else { trimmed.to_string() };
    if is_reserved(&name) { format!("_{name}") } else { name }
}

/// The stem and the extension (the text from the last dot, the dot included, if that dot isn't the first character and the
/// extension is at most 16 bytes). Otherwise the extension is empty.
fn split_ext(cleaned: &str) -> (&str, &str) {
    match cleaned.rfind('.') {
        Some(dot) if dot > 0 && cleaned.len() - dot <= MAX_EXT_BYTES => cleaned.split_at(dot),
        _ => (cleaned, ""),
    }
}

/// The `n`-th candidate for a cleaned name: no suffix for 0, ` (1)`, ` (2)`, … before the extension, with the stem cut to the
/// longest prefix of whole code points that keeps the whole name within 200 bytes. After a cut the stem loses trailing dots and
/// spaces, an empty one becomes `file`, and a reserved one gets a `_` in front.
pub fn candidate(cleaned: &str, n: u32) -> String {
    let (stem, ext) = split_ext(cleaned);
    let suffix = if n == 0 { String::new() } else { format!(" ({n})") };
    let budget = MAX_NAME_BYTES.saturating_sub(suffix.len() + ext.len());
    let mut end = 0;
    for (i, c) in stem.char_indices() {
        if i + c.len_utf8() > budget {
            break;
        }
        end = i + c.len_utf8();
    }
    let mut head = stem[..end].to_string();
    if end < stem.len() {
        head = head.trim_end_matches(['.', ' ']).to_string();
        if head.is_empty() {
            head = "file".to_string();
        }
        if is_reserved(&head) {
            head.insert(0, '_');
        }
    }
    format!("{head}{suffix}{ext}")
}

/// Names compare case-insensitively (Unicode default lowercase mapping, no normalisation), because Windows does.
pub fn fold(name: &str) -> String {
    name.to_lowercase()
}

/// `cleanFileName(input, existing)` of the vectors: the first candidate that isn't in `existing`.
pub fn clean_file_name(input: &str, existing: &[String]) -> String {
    let taken: HashSet<String> = existing.iter().map(|n| fold(n)).collect();
    first_free(&clean(input), &taken)
}

fn first_free(cleaned: &str, taken: &HashSet<String>) -> String {
    (0..).map(|n| candidate(cleaned, n)).find(|c| !taken.contains(&fold(c))).expect("an unbounded range always ends at a free name")
}

/// The first candidate from number `from` (0 is the plain name) up to [`MAX_CANDIDATES`] that isn't in `taken`, with its number.
fn first_free_from(cleaned: &str, taken: &HashSet<String>, from: u32) -> Option<(u32, String)> {
    (from..MAX_CANDIDATES).map(|n| (n, candidate(cleaned, n))).find(|(_, c)| !taken.contains(&fold(c)))
}

/// What a create or a move said.
#[derive(Debug)]
pub enum CreateError {
    /// The name is taken (`ERROR_FILE_EXISTS`, `ERROR_ALREADY_EXISTS`): a collision, so the next suffix is tried.
    Exists,
    /// Anything else (permission, a name the filesystem refuses, an I/O error): the file fails with `internal`.
    Other(String),
}

/// The create-new loop of the spec: pick the first candidate that is not in `existing`, try to create it, and on "already
/// exists" add it to `existing` and try the next suffix. `create` does the real create (or move) and is the real collision check.
/// `Err` carries the reason of a failure that is not a collision.
pub fn create_new<T>(
    input: &str,
    existing: &mut HashSet<String>,
    mut create: impl FnMut(&str) -> Result<T, CreateError>,
) -> Result<(String, T), String> {
    let cleaned = clean(input);
    let mut from = 0;
    while let Some((n, name)) = first_free_from(&cleaned, existing, from) {
        match create(&name) {
            Ok(made) => {
                existing.insert(fold(&name));
                return Ok((name, made));
            }
            Err(CreateError::Exists) => {
                existing.insert(fold(&name));
                from = n + 1;
            }
            Err(CreateError::Other(reason)) => return Err(reason),
        }
    }
    Err(format!("no free name in {MAX_CANDIDATES} tries"))
}

// ----- the folder -------------------------------------------------------------------------------------------------

pub const PART_EXT: &str = ".part";

/// The names already in a folder, folded, for `existing`. A `name.ext.part` file claims `name.ext` as well.
pub fn existing_names(dir: &Path) -> HashSet<String> {
    let mut names = HashSet::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return names;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let folded = fold(&name);
        if let Some(claimed) = folded.strip_suffix(PART_EXT) {
            names.insert(claimed.to_string());
        }
        names.insert(folded);
    }
    names
}

pub fn part_path(dir: &Path, final_name: &str) -> PathBuf {
    dir.join(format!("{final_name}{PART_EXT}"))
}

fn is_exists(e: &io::Error) -> bool {
    // ERROR_FILE_EXISTS is 80 and ERROR_ALREADY_EXISTS is 183: `CREATE_NEW` reports the first, a move reports the second.
    e.kind() == io::ErrorKind::AlreadyExists || matches!(e.raw_os_error(), Some(80 | 183))
}

/// Creates `name.part` in `dir` with create-new semantics (`CREATE_NEW`). The file is opened for writing.
pub fn create_part(dir: &Path, final_name: &str) -> Result<std::fs::File, CreateError> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(part_path(dir, final_name))
        .map_err(|e| if is_exists(&e) { CreateError::Exists } else { CreateError::Other(e.to_string()) })
}

/// Moves `from` to `to` without replacing anything: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`. (`std::fs::rename`
/// replaces an existing target on Windows, which would overwrite a file that appeared under the final name meanwhile.)
pub fn move_no_replace(from: &Path, to: &Path) -> Result<(), CreateError> {
    #[cfg(windows)]
    {
        use windows::Win32::Storage::FileSystem::{MOVE_FILE_FLAGS, MoveFileExW};
        use windows::core::HSTRING;
        // A virus scanner (or the search indexer) often has a file that was just closed open for a moment, and then the move fails with
        // "in use" (32, 33) or "access denied" (5). That passes, so the move is tried again for about a second before it is given up.
        let mut attempt = 0;
        // `MoveFileExW` has no long-path support of its own: a path over 259 characters needs the `\\?\` prefix.
        let (from, to) = (HSTRING::from(crate::paths::long_path(from).as_os_str()), HSTRING::from(crate::paths::long_path(to).as_os_str()));
        loop {
            // SAFETY: both paths are NUL-terminated wide strings that outlive the call. No flag: the move fails if `to` exists.
            let result = unsafe { MoveFileExW(&from, &to, MOVE_FILE_FLAGS(0)) };
            let Err(e) = result else {
                return Ok(());
            };
            let code = e.code().0 as u32 & 0xFFFF;
            if matches!(code, 80 | 183) {
                return Err(CreateError::Exists);
            }
            attempt += 1;
            if matches!(code, 5 | 32 | 33) && attempt < 20 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                continue;
            }
            return Err(CreateError::Other(e.to_string()));
        }
    }
    #[cfg(not(windows))]
    {
        // Other platforms: a hard link fails if the target exists, and the old name is removed afterwards.
        std::fs::hard_link(from, to).map_err(|e| if is_exists(&e) { CreateError::Exists } else { CreateError::Other(e.to_string()) })?;
        std::fs::remove_file(from).map_err(|e| CreateError::Other(e.to_string()))
    }
}

/// Moves `name.part` to its final name, never replacing: if something has appeared under the final name in the meantime (or the
/// filesystem thinks another name is the same), the next suffix is taken. Returns the name it ended up with.
pub fn finish_part(dir: &Path, first_choice: &str, existing: &mut HashSet<String>) -> Result<String, String> {
    let from = part_path(dir, first_choice);
    create_new(first_choice, existing, |name| move_no_replace(&from, &dir.join(name))).map(|(name, ())| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_steps_in_order() {
        assert_eq!(clean("a/b\\c.txt"), "c.txt");
        assert_eq!(clean("dir/"), "file");
        assert_eq!(clean("a<b>c:d\"e|f?g*h"), "a_b_c_d_e_f_g_h");
        assert_eq!(clean("photo\u{202e}gpj.exe"), "photo_gpj.exe");
        assert_eq!(clean("a\u{0}b\u{9f}c\u{7f}"), "a_b_c_");
        assert_eq!(clean("name. . "), "name");
        assert_eq!(clean("..."), "file");
        assert_eq!(clean("con.txt"), "_con.txt");
        assert_eq!(clean("NUL.tar.gz"), "_NUL.tar.gz");
        assert_eq!(clean("Com1"), "_Com1");
        assert_eq!(clean("LPT0.txt"), "_LPT0.txt");
        assert_eq!(clean("COM\u{b2}"), "_COM\u{b2}");
        assert_eq!(clean("console.txt"), "console.txt");
        assert_eq!(clean("COM10"), "COM10");
        assert_eq!(clean(".con"), ".con");
        assert_eq!(clean("CON  .txt"), "_CON  .txt", "trailing spaces of the part before the dot are ignored");
    }

    #[test]
    fn collisions_add_a_suffix_before_the_extension() {
        assert_eq!(clean_file_name("photo.jpg", &s(&["photo.jpg"])), "photo (1).jpg");
        assert_eq!(clean_file_name("photo.jpg", &s(&["PHOTO.JPG", "photo (1).jpg"])), "photo (2).jpg");
        assert_eq!(clean_file_name("notes", &s(&["notes"])), "notes (1)");
        assert_eq!(clean_file_name(".bashrc", &s(&[".bashrc"])), ".bashrc (1)");
        assert_eq!(clean_file_name(".bashrc.bak", &s(&[".bashrc.bak"])), ".bashrc (1).bak");
    }

    #[test]
    fn the_limit_counts_bytes_and_cuts_on_code_points() {
        let long = "日".repeat(100); // 300 bytes
        let got = clean_file_name(&format!("{long}.jpg"), &[]);
        assert!(got.len() <= MAX_NAME_BYTES);
        assert_eq!(got, format!("{}.jpg", "日".repeat(65)), "65 characters are 195 bytes, plus .jpg is 199");
        // The suffix counts too.
        let got1 = clean_file_name(&format!("{long}.jpg"), std::slice::from_ref(&got));
        assert!(got1.len() <= MAX_NAME_BYTES);
        assert!(got1.ends_with(" (1).jpg"));
        // A cut that leaves a reserved name, or trailing dots, is repaired.
        assert_eq!(clean_file_name(&format!("{}x", "a".repeat(200)), &[]).len(), 200);
        let dotted = format!("{}.{}", "a".repeat(190), "b".repeat(30));
        assert!(clean_file_name(&dotted, &[]).len() <= MAX_NAME_BYTES);
    }

    #[test]
    fn create_new_takes_the_next_suffix_on_exists_and_stops_on_other_failures() {
        let on_disk = ["photo.jpg", "photo (1).jpg"];
        let mut existing = HashSet::new();
        let (name, ()) = create_new("photo.jpg", &mut existing, |n| {
            if on_disk.iter().any(|d| fold(d) == fold(n)) { Err(CreateError::Exists) } else { Ok(()) }
        })
        .unwrap();
        assert_eq!(name, "photo (2).jpg");
        let mut existing = HashSet::new();
        let failed = create_new("photo.jpg", &mut existing, |_| -> Result<(), _> { Err(CreateError::Other("denied".into())) });
        assert_eq!(failed.unwrap_err(), "denied");
    }

    #[test]
    fn the_hundredth_candidate_is_the_last_one_tried() {
        // The plain name and ` (1)` to ` (99)` are the 100 candidates. All taken: the file fails.
        let taken: HashSet<String> = (0..MAX_CANDIDATES).map(|n| fold(&candidate("photo.jpg", n))).collect();
        assert_eq!(taken.len(), 100);
        let mut tries = 0;
        let mut existing = taken.clone();
        let failed = create_new("photo.jpg", &mut existing, |_| -> Result<(), CreateError> {
            tries += 1;
            Ok(())
        });
        assert!(failed.is_err() && tries == 0, "nothing is even tried when every candidate is known to be taken: {failed:?}");
        // One free, the last one: it is used.
        let mut existing = taken.clone();
        existing.remove(&fold("photo (99).jpg"));
        assert_eq!(create_new("photo.jpg", &mut existing, |_| Ok::<(), CreateError>(())).unwrap().0, "photo (99).jpg");
        // A create that says "exists" 100 times in a row (a stale list) also stops, and tries each candidate once.
        let mut attempts = Vec::new();
        let failed = create_new("a.txt", &mut HashSet::new(), |name| -> Result<(), CreateError> {
            attempts.push(name.to_string());
            Err(CreateError::Exists)
        });
        assert!(failed.is_err());
        assert_eq!(attempts.len(), 100);
        assert_eq!((attempts[0].as_str(), attempts[1].as_str(), attempts[99].as_str()), ("a.txt", "a (1).txt", "a (99).txt"));
    }

    #[cfg(windows)]
    #[test]
    fn a_move_over_259_characters_works() {
        let dir = tempfile::tempdir().unwrap();
        let deep = ["d".repeat(70), "e".repeat(70), "f".repeat(70)].iter().fold(dir.path().to_path_buf(), |p, s| p.join(s));
        std::fs::create_dir_all(&deep).unwrap();
        let name = format!("{}.bin", "n".repeat(60));
        std::fs::write(part_path(&deep, &name), "data").unwrap();
        assert!(deep.join(&name).to_string_lossy().len() > 259, "{}", deep.join(&name).display());
        assert_eq!(finish_part(&deep, &name, &mut HashSet::new()).unwrap(), name);
        assert_eq!(std::fs::read_to_string(deep.join(&name)).unwrap(), "data");
        // And a collision at that length still takes the next suffix.
        std::fs::write(part_path(&deep, &name), "second").unwrap();
        assert_eq!(finish_part(&deep, &name, &mut HashSet::new()).unwrap(), format!("{} (1).bin", "n".repeat(60)));
    }

    #[test]
    fn the_real_folder_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "mine").unwrap();
        // The create call itself says "already exists" for a name that is taken.
        assert!(create_part(dir.path(), "x.txt").is_ok());
        assert!(matches!(create_part(dir.path(), "x.txt"), Err(CreateError::Exists)));
        // A .part file claims its final name.
        let existing = existing_names(dir.path());
        assert!(existing.contains("x.txt") && existing.contains("x.txt.part") && existing.contains("a.txt"));
        // The move takes the next suffix when the final name appeared meanwhile, and a file that was there is untouched.
        std::fs::write(part_path(dir.path(), "a.txt"), "theirs").unwrap();
        let mut existing = HashSet::new();
        let name = finish_part(dir.path(), "a.txt", &mut existing).unwrap();
        assert_eq!(name, "a (1).txt");
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(dir.path().join("a (1).txt")).unwrap(), "theirs");
        assert!(!part_path(dir.path(), "a.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_part_file_that_something_else_holds_for_a_moment_is_moved_when_it_lets_go() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let part = part_path(dir.path(), "held.bin");
        std::fs::write(&part, "data").unwrap();
        // A scanner has the file open with no sharing at all, and lets go after 300 ms.
        let holder = std::fs::OpenOptions::new().read(true).share_mode(0).open(&part).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            drop(holder);
        });
        let name = finish_part(dir.path(), "held.bin", &mut HashSet::new()).expect("the move waits for the scanner");
        release.join().unwrap();
        assert_eq!(name, "held.bin");
        assert_eq!(std::fs::read_to_string(dir.path().join("held.bin")).unwrap(), "data");
    }

    #[test]
    fn the_move_does_not_replace_a_name_the_filesystem_treats_as_the_same() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("PHOTO.JPG"), "mine").unwrap();
        std::fs::write(part_path(dir.path(), "photo.jpg"), "theirs").unwrap();
        let name = finish_part(dir.path(), "photo.jpg", &mut HashSet::new()).unwrap();
        assert_eq!(name, "photo (1).jpg");
        assert_eq!(std::fs::read_to_string(dir.path().join("PHOTO.JPG")).unwrap(), "mine");
    }
}
