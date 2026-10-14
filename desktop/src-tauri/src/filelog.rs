//! Rotating log files: `kin.log`, then `kin.1.log` to `kin.4.log`, 1 MB each (windows.md, "Data locations").
//!
//! Nothing here knows about secrets, so callers must never pass a private key or a pairing token to a log macro.
//! Pairing logs the result of an attempt and never the token or the URL that holds it.
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::Result;

pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
pub const FILES_KEPT: usize = 5;
const FILE_NAME: &str = "kin";

struct Rotating {
    dir: PathBuf,
    file: File,
    size: u64,
    max_bytes: u64,
}

impl Rotating {
    fn path(dir: &Path, n: usize) -> PathBuf {
        if n == 0 { dir.join(format!("{FILE_NAME}.log")) } else { dir.join(format!("{FILE_NAME}.{n}.log")) }
    }

    fn open(dir: &Path, max_bytes: u64) -> Result<Rotating> {
        fs::create_dir_all(dir)?;
        let file = OpenOptions::new().create(true).append(true).open(Rotating::path(dir, 0))?;
        let size = file.metadata()?.len();
        Ok(Rotating { dir: dir.to_path_buf(), file, size, max_bytes })
    }

    fn rotate(&mut self) -> std::io::Result<()> {
        // kin.3.log -> kin.4.log, ..., kin.log -> kin.1.log. `rename` replaces the target, which is what we want here.
        let _ = fs::remove_file(Rotating::path(&self.dir, FILES_KEPT - 1));
        for n in (0..FILES_KEPT - 1).rev() {
            let from = Rotating::path(&self.dir, n);
            if from.exists() {
                fs::rename(&from, Rotating::path(&self.dir, n + 1))?;
            }
        }
        self.file = OpenOptions::new().create(true).append(true).open(Rotating::path(&self.dir, 0))?;
        self.size = 0;
        Ok(())
    }

    fn write_line(&mut self, line: &str) {
        if self.size + line.len() as u64 > self.max_bytes && self.size > 0 {
            let _ = self.rotate();
        }
        if self.file.write_all(line.as_bytes()).is_ok() {
            self.size += line.len() as u64;
        }
    }
}

pub struct FileLogger {
    out: Mutex<Rotating>,
}

impl FileLogger {
    pub fn open(dir: &Path, max_bytes: u64) -> Result<FileLogger> {
        Ok(FileLogger { out: Mutex::new(Rotating::open(dir, max_bytes)?) })
    }
}

impl Log for FileLogger {
    // The app's own modules log at Info, everything else (rustls, hyper, ...) only warnings and errors.
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        let own = metadata.target().starts_with("kin");
        metadata.level() <= if own { Level::Info } else { Level::Warn }
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}: {}\n", utc_timestamp(crate::encoding::unix_ms()), record.level(), record.target(), record.args());
        if let Ok(mut out) = self.out.lock() {
            out.write_line(&line);
        }
    }

    fn flush(&self) {
        if let Ok(mut out) = self.out.lock() {
            let _ = out.file.flush();
        }
    }
}

/// Installs the file logger as the global logger. Only the first call in a process has any effect.
pub fn init(dir: &Path) -> Result<()> {
    let logger = FileLogger::open(dir, MAX_FILE_BYTES)?;
    if log::set_boxed_logger(Box::new(logger)).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
    Ok(())
}

/// Logs panics with their location, so a crashed task leaves a line in the file.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        log::error!("panic: {info}");
    }));
}

/// `2026-10-14T12:34:56.789Z`
pub fn utc_timestamp(unix_ms: u64) -> String {
    let (secs, ms) = (unix_ms / 1000, unix_ms % 1000);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{ms:03}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(utc_timestamp(1_760_443_200_123), "2025-10-14T12:00:00.123Z");
        assert_eq!(utc_timestamp(1_782_864_000_000), "2026-07-01T00:00:00.000Z");
        assert_eq!(utc_timestamp(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn keeps_five_files_of_at_most_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = Rotating::open(dir.path(), 100).unwrap();
        for i in 0..40 {
            r.write_line(&format!("line number {i:04} with some padding to make it long\n"));
        }
        let mut names: Vec<_> = fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().into_string().unwrap()).collect();
        names.sort();
        assert_eq!(names, ["kin.1.log", "kin.2.log", "kin.3.log", "kin.4.log", "kin.log"]);
        for n in &names {
            assert!(fs::metadata(dir.path().join(n)).unwrap().len() <= 100, "{n} is over the limit");
        }
        // The newest line is in kin.log, and the oldest ones are gone.
        assert!(fs::read_to_string(dir.path().join("kin.log")).unwrap().contains("0039"));
        let all: String = names.iter().map(|n| fs::read_to_string(dir.path().join(n)).unwrap()).collect();
        assert!(!all.contains("0000"));
    }
}
