use std::collections::VecDeque;

use crate::filter::Expr;
use crate::logcat::LogEntry;

const RING_CAP: usize = 100_000;
const EVICT_BATCH: usize = RING_CAP / 5; // 20_000

/// Ring buffer for log entries with a hard upper bound.
///
/// When the buffer is full, the oldest `EVICT_BATCH` entries are dropped.
/// A filtered view (`Vec<usize>`) stores indices into `entries` for entries
/// matching the current filter, avoiding data copies during rendering.
pub struct LogBuffer {
    entries: VecDeque<LogEntry>,
    /// Indices into `entries` that pass the current filter.
    filtered: Vec<usize>,
    /// Total number of lines ever parsed (including dropped ones).
    pub total_parsed: u64,
    /// Total number of entries dropped due to buffer overflow.
    pub total_dropped: u64,
    /// Number of entries filtered out by the current filter (this filter session).
    pub total_filtered_out: u64,
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            filtered: Vec::new(),
            total_parsed: 0,
            total_dropped: 0,
            total_filtered_out: 0,
        }
    }

    /// Push a new entry. If the buffer is full, evict the oldest batch first.
    ///
    /// If `filter` is `Some`, the entry is tested against it; if it passes,
    /// the entry's index is appended to `filtered`.
    /// If `filter` is `None`, all entries pass.
    pub fn push(&mut self, entry: LogEntry, filter: Option<&Expr>) {
        self.total_parsed += 1;

        // Evict if full
        if self.entries.len() >= RING_CAP {
            self.entries.drain(0..EVICT_BATCH);
            self.total_dropped += EVICT_BATCH as u64;
            self.entries.shrink_to_fit();
            // Rebuild all indices after eviction
            self.rebuild_filtered(filter);
        }

        let idx = self.entries.len();
        let passes = match filter {
            Some(expr) => expr.evaluate(&entry),
            None => true,
        };

        if !passes {
            self.total_filtered_out += 1;
        }

        self.entries.push_back(entry);

        if passes {
            self.filtered.push(idx);
        }
    }

    /// Rebuild the filtered index from scratch.
    pub fn rebuild_filtered(&mut self, filter: Option<&Expr>) {
        let mut new_filtered = Vec::with_capacity(self.filtered.len());
        let mut filtered_out = 0u64;

        for (i, entry) in self.entries.iter().enumerate() {
            let passes = match filter {
                Some(expr) => expr.evaluate(entry),
                None => true,
            };
            if passes {
                new_filtered.push(i);
            } else {
                filtered_out += 1;
            }
        }

        self.filtered = new_filtered;
        self.total_filtered_out = filtered_out;
    }

    /// Clear all entries and reset counters.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.entries.shrink_to_fit();
        self.filtered.clear();
        self.filtered.shrink_to_fit();
        self.total_parsed = 0;
        self.total_dropped = 0;
        self.total_filtered_out = 0;
    }

    /// Number of entries currently in the buffer.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Number of entries passing the current filter.
    pub fn filtered_len(&self) -> usize {
        self.filtered.len()
    }


    /// Get the entry at a given position in the filtered view.
    pub fn get_filtered(&self, pos: usize) -> Option<(usize, &LogEntry)> {
        let idx = *self.filtered.get(pos)?;
        self.entries.get(idx).map(|e| (idx, e))
    }

}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logcat::LogLevel;

    fn make_entry(tag: &str, msg: &str) -> LogEntry {
        LogEntry {
            timestamp: chrono::Local::now().naive_local(),
            pid: 0,
            tid: 0,
            level: LogLevel::Info,
            tag: tag.to_string(),
            message: msg.to_string(),
            package: None,
        }
    }

    #[test]
    fn test_push_and_get() {
        let mut buf = LogBuffer::new();
        buf.push(make_entry("A", "one"), None);
        buf.push(make_entry("B", "two"), None);
        assert_eq!(buf.len(), 2);
        assert_eq!(buf.filtered_len(), 2);
        assert_eq!(buf.get_filtered(0).unwrap().1.tag, "A");
        assert_eq!(buf.get_filtered(1).unwrap().1.tag, "B");
    }

    #[test]
    fn test_eviction() {
        let mut buf = LogBuffer::new();
        // Fill beyond RING_CAP
        for i in 0..(RING_CAP + EVICT_BATCH + 10) {
            buf.push(make_entry("T", &format!("msg{}", i)), None);
        }
        // Should have evicted the first EVICT_BATCH, then some more
        assert!(buf.len() <= RING_CAP);
        assert!(buf.total_dropped > 0);
        // The first entries should be gone
        let (_, first) = buf.get_filtered(0).unwrap();
        // Should not be "msg0" — that was evicted
        assert_ne!(first.message, "msg0");
    }

    #[test]
    fn test_clear() {
        let mut buf = LogBuffer::new();
        buf.push(make_entry("A", "one"), None);
        buf.push(make_entry("B", "two"), None);
        buf.clear();
        assert_eq!(buf.len(), 0);
        assert_eq!(buf.filtered_len(), 0);
        assert_eq!(buf.total_parsed, 0);
        assert_eq!(buf.total_dropped, 0);
    }
}
