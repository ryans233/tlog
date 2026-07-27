use chrono::{Datelike, NaiveDateTime};
#[cfg(test)]
use chrono::Timelike;

use regex::Regex;
use std::sync::LazyLock;
/// Log level in order of severity (matching Android log levels).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Verbose = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
}

impl LogLevel {
    pub fn from_char(c: char) -> Option<LogLevel> {
        match c {
            'V' => Some(LogLevel::Verbose),
            'D' => Some(LogLevel::Debug),
            'I' => Some(LogLevel::Info),
            'W' => Some(LogLevel::Warn),
            'E' => Some(LogLevel::Error),
            'F' => Some(LogLevel::Fatal),
            'S' => Some(LogLevel::Fatal), // Silent = Fatal
            _ => None,
        }
    }

    pub fn as_char(self) -> char {
        match self {
            LogLevel::Verbose => 'V',
            LogLevel::Debug => 'D',
            LogLevel::Info => 'I',
            LogLevel::Warn => 'W',
            LogLevel::Error => 'E',
            LogLevel::Fatal => 'F',
        }
    }
}

/// A parsed logcat entry from `-v threadtime` output.
///
/// Format: `MM-DD HH:MM:SS.mmm  PID  TID L TAG: MESSAGE`
/// PID/TID are 5-char right-aligned fields. L is a single letter level.
#[derive(Clone, Debug)]
pub struct LogEntry {
    pub timestamp: NaiveDateTime,
    pub pid: u32,
    pub tid: u32,
    pub level: LogLevel,
    pub tag: String,
    pub message: String,
    /// Resolved package name for this PID (populated by App from PID→package map).
    pub package: Option<String>,
}

pub enum Message {
    /// A newly parsed log entry.
    NewEntry(LogEntry),
    /// Periodic report of cumulative dropped messages (channel full).
    Dropped(u64),
    /// A process was started (from ActivityManager lifecycle parsing).
    ProcessStarted { pid: u32, package: String },
    /// A process died (from ActivityManager lifecycle parsing).
    ProcessDied { pid: u32 },
    /// The logcat subprocess exited.
    LogcatDied,
    /// Periodic UI tick.
    Tick,
}

const MAX_TAG_BYTES: usize = 256;
const MAX_MSG_BYTES: usize = 4096;

/// Trim a string to at most `max_bytes` bytes on a valid UTF-8 char boundary.
fn truncate_to_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Parse a single line of `adb logcat -v threadtime` output.
///
/// Returns `None` for lines that don't match the expected format
/// (e.g., `--------- beginning of ...` divider lines).
///
/// Format: `MM-DD HH:MM:SS.mmm  PID  TID L TAG: MESSAGE`
/// Positions:
///   0-1:   month (2 chars)
///   2:     '-'
///   3-4:   day (2 chars)
///   5:     ' '
///   6-7:   hour (2 chars)
///   8:     ':'
///   9-10:  minute (2 chars)
///   11:    ':'
///   12-13: second (2 chars)
///   14:    '.'
///   15-17: millisecond (3 chars)
///   18:    ' '
///   19-23: PID (5 chars, right-aligned)
///   24:    ' '
///   25-29: TID (5 chars, right-aligned)
///   30:    ' '
///   31:    level char
///   32:    ' '
///   33+:   TAG: MESSAGE
pub fn parse_line(line: &str) -> Option<LogEntry> {
    // Minimum length check: "MM-DD HH:MM:SS.mmm  PID  TID L TAG: MESSAGE"
    // The shortest valid line is approximately 33 + 3 chars (tag: at least 1 char + ": " + message).
    if line.len() < 37 {
        return None;
    }

    let bytes = line.as_bytes();

    // Parse month (positions 0-1)
    if bytes.len() < 2 || !bytes[0].is_ascii_digit() || !bytes[1].is_ascii_digit() {
        return None;
    }
    let month: u32 = ((bytes[0] - b'0') * 10 + (bytes[1] - b'0')) as u32;
    if month == 0 || month > 12 {
        return None;
    }

    // Check dash at pos 2
    if bytes.get(2)? != &b'-' {
        return None;
    }

    // Parse day (positions 3-4)
    if bytes.len() < 5 || !bytes[3].is_ascii_digit() || !bytes[4].is_ascii_digit() {
        return None;
    }
    let day: u32 = ((bytes[3] - b'0') * 10 + (bytes[4] - b'0')) as u32;
    if day == 0 || day > 31 {
        return None;
    }

    // Check space at pos 5
    if bytes.get(5)? != &b' ' {
        return None;
    }

    // Parse hour (6-7)
    if bytes.len() < 8 || !bytes[6].is_ascii_digit() || !bytes[7].is_ascii_digit() {
        return None;
    }
    let hour: u32 = ((bytes[6] - b'0') * 10 + (bytes[7] - b'0')) as u32;
    if hour > 23 {
        return None;
    }

    // Check colon at pos 8
    if bytes.get(8)? != &b':' {
        return None;
    }

    // Parse minute (9-10)
    if bytes.len() < 11 || !bytes[9].is_ascii_digit() || !bytes[10].is_ascii_digit() {
        return None;
    }
    let min: u32 = ((bytes[9] - b'0') * 10 + (bytes[10] - b'0')) as u32;
    if min > 59 {
        return None;
    }

    // Check colon at pos 11
    if bytes.get(11)? != &b':' {
        return None;
    }

    // Parse second (12-13)
    if bytes.len() < 14 || !bytes[12].is_ascii_digit() || !bytes[13].is_ascii_digit() {
        return None;
    }
    let sec: u32 = ((bytes[12] - b'0') * 10 + (bytes[13] - b'0')) as u32;
    if sec > 59 {
        return None;
    }

    // Check dot at pos 14
    if bytes.get(14)? != &b'.' {
        return None;
    }

    // Parse millisecond (15-17)
    if bytes.len() < 18 || !bytes[15].is_ascii_digit() || !bytes[16].is_ascii_digit() || !bytes[17].is_ascii_digit() {
        return None;
    }
    let ms: u32 = ((bytes[15] - b'0') * 100 + (bytes[16] - b'0') * 10 + (bytes[17] - b'0')) as u32;

    // Check space at pos 18
    if bytes.get(18)? != &b' ' {
        return None;
    }

    // Parse PID (positions 19-23, 5 chars right-aligned)
    // PID is exactly 5 chars: 19,20,21,22,23
    if bytes.len() < 24 {
        return None;
    }
    let pid = parse_5char_field(&bytes[19..24])?;

    // Check space at pos 24
    if bytes.get(24)? != &b' ' {
        return None;
    }

    // Parse TID (positions 25-29, 5 chars right-aligned)
    if bytes.len() < 30 {
        return None;
    }
    let tid = parse_5char_field(&bytes[25..30])?;

    // Check space at pos 30
    if bytes.get(30)? != &b' ' {
        return None;
    }

    // Parse level char at pos 31
    let level = LogLevel::from_char(bytes[31] as char)?;

    // Check space at pos 32
    if bytes.get(32)? != &b' ' {
        return None;
    }

    // Parse TAG: MESSAGE (position 33+)
    let rest = &line[33..];

    // Find ": " as TAG/MESSAGE separator
    let colon_space = rest.find(": ")?;
    let tag = truncate_to_boundary(&rest[..colon_space], MAX_TAG_BYTES);
    let message = truncate_to_boundary(&rest[colon_space + 2..], MAX_MSG_BYTES);

    // Build timestamp: use current year since logcat doesn't include it
    let now = chrono::Local::now();
    let ts = NaiveDateTime::new(
        chrono::NaiveDate::from_ymd_opt(now.year(), month, day)?,
        chrono::NaiveTime::from_hms_milli_opt(hour, min, sec, ms)?,
    );

    Some(LogEntry {
        timestamp: ts,
        pid,
        tid,
        level,
        tag: tag.to_owned(),
        message: message.to_owned(),
        package: None,
    })
}

/// Parse a 5-character right-aligned numeric field (PID/TID).
/// Leading spaces are treated as zeros.
fn parse_5char_field(field: &[u8]) -> Option<u32> {
    if field.len() != 5 {
        return None;
    }
    let mut val: u32 = 0;
    for &b in field {
        if b == b' ' {
            val = val.wrapping_mul(10);
        } else if b.is_ascii_digit() {
            val = val.wrapping_mul(10) + (b - b'0') as u32;
        } else {
            return None;
        }
    }
    Some(val)
}


// ── Process lifecycle detection ────────────────────────────────────────────

/// Regex patterns for ActivityManager lifecycle events.
/// Lazily compiled once, reused across all log lines.
static PROC_START_51: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Start proc (\d+):([a-zA-Z0-9._:]+)/").unwrap()
});
static PROC_START_LEGACY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Start proc ([a-zA-Z0-9._:]+) for .*: pid=(\d+)").unwrap()
});
static PROC_KILL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Killing (\d+):([a-zA-Z0-9._:]+)/").unwrap()
});
static PROC_DEATH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^Process ([a-zA-Z0-9._:]+) \(pid (\d+)\) has died").unwrap()
});
static PROC_LEAVE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^No longer want ([a-zA-Z0-9._:]+) \(pid (\d+)\)").unwrap()
});

/// Possible lifecycle event detected in an ActivityManager log line.
#[derive(Debug)]
pub enum LifecycleEvent {
    Started { pid: u32, package: String },
    Died { pid: u32 },
}

/// Parse an ActivityManager log line for process lifecycle events.
///
/// Called for every log line where tag == "ActivityManager".
/// Returns `None` if the line doesn't contain a lifecycle event.
pub fn parse_lifecycle(tag: &str, message: &str) -> Option<LifecycleEvent> {
    if tag != "ActivityManager" {
        return None;
    }

    // Process start (Android 5.1+): "Start proc 12345:com.example/u0a123 for ..."
    if let Some(caps) = PROC_START_51.captures(message) {
        let pid: u32 = caps.get(1)?.as_str().parse().ok()?;
        let package = caps.get(2)?.as_str().to_string();
        return Some(LifecycleEvent::Started { pid, package });
    }

    // Process start (legacy): "Start proc com.example for activity ...: pid=12345 ..."
    if let Some(caps) = PROC_START_LEGACY.captures(message) {
        let package = caps.get(1)?.as_str().to_string();
        let pid: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(LifecycleEvent::Started { pid, package });
    }

    // Process kill: "Killing 12345:com.example/u0a123: reason"
    if let Some(caps) = PROC_KILL.captures(message) {
        let pid: u32 = caps.get(1)?.as_str().parse().ok()?;
        return Some(LifecycleEvent::Died { pid });
    }

    // Process death: "Process com.example (pid 12345) has died"
    if let Some(caps) = PROC_DEATH.captures(message) {
        let pid: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(LifecycleEvent::Died { pid });
    }

    // Process leave: "No longer want com.example (pid 12345): ..."
    if let Some(caps) = PROC_LEAVE.captures(message) {
        let pid: u32 = caps.get(2)?.as_str().parse().ok()?;
        return Some(LifecycleEvent::Died { pid });
    }

    None
}

// ── adb shell ps parser ────────────────────────────────────────────────────

/// Parse a single line of `adb shell ps` (or `adb shell ps -A`) output.
///
/// Format (column headers, variable whitespace):
/// ```text
/// USER           PID  PPID     VSZ    RSS WCHAN            ADDR S NAME
/// u0_a123       12345   789 12345678 123456 do_epoll_wait      0 S com.example
/// ```
///
/// Returns `(pid, process_name)`.
pub fn parse_ps_line(line: &str) -> Option<(u32, String)> {
    // Skip header line
    if line.starts_with("USER") {
        return None;
    }

    // Split by whitespace, last column is NAME
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 9 {
        return None;
    }

    // PID is the 2nd column (index 1)
    let pid: u32 = parts[1].parse().ok()?;

    // NAME is the last column
    let name = parts[parts.len() - 1].to_string();

    Some((pid, name))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_normal_line() {
        let line = "07-27 10:15:30.123  1234  5678 D MyTag: hello world";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.pid, 1234);
        assert_eq!(entry.tid, 5678);
        assert_eq!(entry.level, LogLevel::Debug);
        assert_eq!(entry.tag, "MyTag");
        assert_eq!(entry.message, "hello world");
        assert_eq!(entry.timestamp.month(), 7);
        assert_eq!(entry.timestamp.day(), 27);
        assert_eq!(entry.timestamp.hour(), 10);
        assert_eq!(entry.timestamp.minute(), 15);
        assert_eq!(entry.timestamp.second(), 30);
        assert_eq!(entry.timestamp.and_utc().timestamp_subsec_millis(), 123);
    }

    #[test]
    fn test_parse_zero_pid() {
        let line = "07-27 10:15:30.123     0     0 W System: boot";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.pid, 0);
        assert_eq!(entry.tid, 0);
        assert_eq!(entry.level, LogLevel::Warn);
    }

    #[test]
    fn test_parse_large_pid_tid() {
        let line = "07-27 10:15:30.123 32767 32767 E Tag: msg";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.pid, 32767);
        assert_eq!(entry.tid, 32767);
        assert_eq!(entry.level, LogLevel::Error);
    }

    #[test]
    fn test_parse_empty_message() {
        let line = "07-27 10:15:30.123     0     0 V Tag: ";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.message, "");
    }

    #[test]
    fn test_parse_fatal_level() {
        let line = "07-27 10:15:30.123     0     0 F Crash: FATAL EXCEPTION";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.level, LogLevel::Fatal);
    }

    #[test]
    fn test_parse_silent_level() {
        let line = "07-27 10:15:30.123     0     0 S Tag: silent";
        let entry = parse_line(line).expect("should parse");
        assert_eq!(entry.level, LogLevel::Fatal);
    }

    #[test]
    fn test_reject_beginning_line() {
        let line = "--------- beginning of main";
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn test_reject_beginning_system() {
        let line = "--------- beginning of system";
        assert!(parse_line(line).is_none());
    }

    #[test]
    fn test_reject_garbage() {
        assert!(parse_line("").is_none());
        assert!(parse_line("hello world").is_none());
        assert!(parse_line("01-01 00:00:00.000").is_none());
    }

    #[test]
    fn test_truncate_to_boundary() {
        let s = "hello world";
        assert_eq!(truncate_to_boundary(s, 5), "hello");
        assert_eq!(truncate_to_boundary(s, 100), "hello world");
        // Test at a non-boundary (multi-byte char)
        let s2 = "héllo"; // é is 2 bytes
        assert_eq!(truncate_to_boundary(s2, 1), "h"); // can't split é
        assert_eq!(truncate_to_boundary(s2, 2), "h");
        assert_eq!(truncate_to_boundary(s2, 3), "hé");
    }
}
