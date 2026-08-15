use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Persisted named filters live next to `config.conf`.
///
/// File format (one entry per line, tab-separated):
/// ```text
/// errors<TAB>tag:MainActivity level:ERROR
/// ```
/// Names and expressions must not contain tabs or newlines; malformed lines
/// are ignored on load so a hand-edited file can never crash the viewer.
pub fn config_path() -> Option<PathBuf> {
    crate::config::config_path().and_then(|conf| conf.parent().map(|dir| dir.join("filters.conf")))
}

/// Load saved filters, tolerating missing files and malformed lines.
pub fn load() -> BTreeMap<String, String> {
    match config_path() {
        Some(path) => load_from(&path),
        None => BTreeMap::new(),
    }
}

pub fn load_from(path: &Path) -> BTreeMap<String, String> {
    let mut filters = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return filters;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, expr)) = line.split_once('\t') else {
            continue;
        };
        let (name, expr) = (name.trim(), expr.trim());
        if name.is_empty()
            || expr.is_empty()
            || name.contains(['\t', '\n', '\r'])
            || expr.contains(['\t', '\n', '\r'])
        {
            continue;
        }
        filters.insert(name.to_string(), expr.to_string());
    }
    filters
}

/// Persist all saved filters to the config directory.
pub fn save(filters: &BTreeMap<String, String>) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no config directory",
        ));
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    save_to(&path, filters)
}

pub fn save_to(path: &Path, filters: &BTreeMap<String, String>) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    for (name, expr) in filters {
        // Entries that would corrupt the tab-separated format are skipped.
        if name.is_empty()
            || expr.is_empty()
            || name.contains(['\t', '\n', '\r'])
            || expr.contains(['\t', '\n', '\r'])
        {
            continue;
        }
        writeln!(file, "{}\t{}", name, expr)?;
    }
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tlog-named-filters-test-{}-{}.conf",
            std::process::id(),
            name
        ))
    }

    #[test]
    fn test_save_load_roundtrip_preserves_special_chars() {
        let path = temp_path("roundtrip");
        let filters = BTreeMap::from([
            ("errors".to_string(), "tag:Foo level:ERROR".to_string()),
            (
                "equals".to_string(),
                "message:key=value | tag~:^Main.*".to_string(),
            ),
        ]);
        save_to(&path, &filters).expect("save should succeed");
        assert_eq!(load_from(&path), filters);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_load_ignores_malformed_and_missing() {
        let path = temp_path("malformed");
        std::fs::write(
            &path,
            "# comment\n\nno-tab-here\nerrors\tlevel:ERROR\n\torphan\ndupe\ttag:A\ndupe\ttag:B\n",
        )
        .expect("write should succeed");
        let filters = load_from(&path);
        assert_eq!(filters.len(), 2);
        assert_eq!(
            filters.get("errors").map(String::as_str),
            Some("level:ERROR")
        );
        assert_eq!(filters.get("dupe").map(String::as_str), Some("tag:B"));
        let _ = std::fs::remove_file(&path);

        let missing = temp_path("missing");
        let _ = std::fs::remove_file(&missing);
        assert!(load_from(&missing).is_empty());
    }

    #[test]
    fn test_save_skips_entries_that_would_corrupt_format() {
        let path = temp_path("skip");
        let filters = BTreeMap::from([
            ("ok".to_string(), "tag:Main".to_string()),
            ("bad\tname".to_string(), "tag:Bad".to_string()),
            ("empty-expr".to_string(), String::new()),
        ]);
        save_to(&path, &filters).expect("save should succeed");
        let loaded = load_from(&path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("ok").map(String::as_str), Some("tag:Main"));
        let _ = std::fs::remove_file(&path);
    }
}
