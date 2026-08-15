use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use chrono::Local;

use crate::buffer::LogBuffer;

/// Default export filename in the current working directory:
/// `tlog-export-YYYYMMDD-HHMMSS.mmm.log`.
pub fn default_export_path() -> PathBuf {
    let stamp = Local::now().format("%Y%m%d-%H%M%S%.3f");
    PathBuf::from(format!("tlog-export-{}.log", stamp))
}

/// Pick a path that does not overwrite an existing file.
///
/// If `base` exists, `-1`, `-2`, ... is appended before the extension.
pub fn available_export_path(base: &Path) -> PathBuf {
    if !base.exists() {
        return base.to_path_buf();
    }
    let parent = base.parent().unwrap_or_else(|| Path::new("."));
    let stem = base
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tlog-export");
    let ext = base.extension().and_then(|e| e.to_str()).unwrap_or("log");
    for i in 1..10_000u32 {
        let candidate = parent.join(format!("{stem}-{i}.{ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    // Extremely unlikely: fall back to the original path.
    base.to_path_buf()
}

/// Write the current filtered view in canonical `threadtime` format.
///
/// The exported file can be replayed with `tlog --file <path>`.
pub fn write_filtered(buffer: &LogBuffer, path: &Path) -> std::io::Result<usize> {
    let file = std::fs::File::create(path)?;
    let mut writer = BufWriter::new(file);
    let mut written = 0usize;
    for pos in 0..buffer.filtered_len() {
        if let Some((_, entry)) = buffer.get_filtered(pos) {
            writeln!(writer, "{}", entry.threadtime_string())?;
            written += 1;
        }
    }
    writer.flush()?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logcat::LogEntry;

    fn make_entry(tag: &str, msg: &str) -> LogEntry {
        LogEntry {
            timestamp: chrono::Local::now().naive_local(),
            pid: 1234,
            tid: 5678,
            level: crate::logcat::LogLevel::Info,
            tag: tag.to_string(),
            message: msg.to_string(),
            package: None,
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tlog-export-test-{}-{}.log",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn test_write_filtered_exports_canonical_lines() {
        let path = temp_path("write");
        let _ = std::fs::remove_file(&path);
        let mut buffer = LogBuffer::new();
        buffer.push(make_entry("TagA", "hello"), None);
        buffer.push(make_entry("TagB", "world"), None);
        let written = write_filtered(&buffer, &path).expect("export should succeed");
        assert_eq!(written, 2);
        let text = std::fs::read_to_string(&path).expect("export readable");
        assert_eq!(text.lines().count(), 2, "one canonical line per entry");
        // Exported lines stay parseable by the same threadtime parser.
        for line in text.lines() {
            assert!(crate::logcat::parse_line(line).is_some(), "got {line:?}");
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_available_export_path_never_overwrites() {
        let path = temp_path("unique");
        std::fs::write(&path, "existing").expect("write should succeed");
        let next = available_export_path(&path);
        assert_ne!(next, path);
        assert!(!next.exists());
        std::fs::write(&next, "taken").expect("write should succeed");
        let third = available_export_path(&path);
        assert_ne!(third, path);
        assert_ne!(third, next);
        assert!(!third.exists());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&next);
        let _ = std::fs::remove_file(&third);
    }
}
