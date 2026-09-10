use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Maximum size of the rejected-log file before it is rotated.
const MAX_REJECTED_LOG_BYTES: u64 = 1024 * 1024;

/// Maximum bytes of a single unparsed line kept in the rejected log.
///
/// Truncation happens on a UTF-8 char boundary, so a multi-byte character
/// straddling the limit can never panic the reader task.
const MAX_UNPARSED_BYTES: usize = 200;

/// Optional debug log for unparsed lines and reader diagnostics.
///
/// Disabled by default. When enabled (`--debug` or `TLOG_DEBUG`), the file is
/// written to the tlog config directory and rotated once it reaches
/// [`MAX_REJECTED_LOG_BYTES`], so it no longer grows without bound in the CWD.
#[derive(Clone, Debug)]
pub struct RejectedLog {
    enabled: bool,
    path: Option<PathBuf>,
}

impl RejectedLog {
    pub fn new(enabled: bool) -> Self {
        let path = if enabled {
            crate::config::config_path()
                .and_then(|conf| conf.parent().map(|dir| dir.join("tlog-rejected.log")))
        } else {
            None
        };
        Self {
            enabled: enabled && path.is_some(),
            path,
        }
    }

    /// Append a `[tlog] ...` diagnostic line.
    pub fn log(&self, msg: &str) {
        self.append(&format!("[tlog] {}", msg));
    }

    /// Append a rejected line, truncating long lines to
    /// [`MAX_UNPARSED_BYTES`] bytes on a UTF-8 char boundary.
    pub fn unparsed(&self, line: &str) {
        if line.len() > MAX_UNPARSED_BYTES {
            self.append(&format!(
                "[unparsed len={}] {}...",
                line.len(),
                crate::logcat::truncate_to_boundary(line, MAX_UNPARSED_BYTES)
            ));
        } else {
            self.append(line);
        }
    }

    fn append(&self, line: &str) {
        let Some(path) = self.enabled.then_some(self.path.as_ref()).flatten() else {
            return;
        };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = rotate_if_needed(path);
        let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
            return;
        };
        let _ = writeln!(file, "{}", line);
    }
}

fn rotate_if_needed(path: &Path) -> std::io::Result<()> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(());
    };
    if meta.len() < MAX_REJECTED_LOG_BYTES {
        return Ok(());
    }
    let old = path.with_extension("old.log");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(path, &old)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tlog-rejected-test-{}-{}.log",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn test_disabled_log_does_not_write() {
        let path = temp_path("disabled");
        let _ = std::fs::remove_file(&path);
        let log = RejectedLog {
            enabled: false,
            path: Some(path.clone()),
        };
        log.log("hidden");
        log.unparsed("hidden");
        assert!(!path.exists());
    }

    #[test]
    fn test_unparsed_truncates_long_lines() {
        let path = temp_path("truncate");
        let _ = std::fs::remove_file(&path);
        let log = RejectedLog {
            enabled: true,
            path: Some(path.clone()),
        };
        let long = "x".repeat(250);
        log.unparsed(&long);
        let text = std::fs::read_to_string(&path).expect("log written");
        assert!(text.starts_with("[unparsed len=250] "));
        assert!(text.ends_with("...\n"));
        assert!(text.len() < 300);

        log.unparsed("short");
        let text = std::fs::read_to_string(&path).expect("log written");
        assert!(text.contains("short\n"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_unparsed_truncates_multibyte_on_char_boundary() {
        let path = temp_path("truncate-multibyte");
        let _ = std::fs::remove_file(&path);
        let log = RejectedLog {
            enabled: true,
            path: Some(path.clone()),
        };
        // 100 three-byte characters: byte 200 falls inside a character, so a
        // raw byte slice would panic the reader task here.
        let long = "中".repeat(100);
        log.unparsed(&long);
        let text = std::fs::read_to_string(&path).expect("log written");
        assert!(text.starts_with("[unparsed len=300] "), "{text:?}");
        assert!(text.ends_with("...\n"), "{text:?}");
        assert_eq!(text.matches('中').count(), 66, "{text:?}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_rotation_keeps_bounded_size() {
        let path = temp_path("rotate");
        let _ = std::fs::remove_file(&path);
        let old_path = path.with_extension("old.log");
        let _ = std::fs::remove_file(&old_path);
        let log = RejectedLog {
            enabled: true,
            path: Some(path.clone()),
        };

        // Write enough to cross the 1 MiB threshold at least once.
        let chunk = "x".repeat(1024);
        for _ in 0..(MAX_REJECTED_LOG_BYTES as usize / chunk.len() + 2) {
            log.append(&chunk);
        }
        assert!(path.exists());
        assert!(old_path.exists(), "oversized log should have been rotated");
        assert!(std::fs::metadata(&path).expect("new log").len() < MAX_REJECTED_LOG_BYTES);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&old_path);
    }
}
