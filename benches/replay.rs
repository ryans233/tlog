//! Lightweight regression benchmarks for tlog's hot paths.
//!
//! tlog is a binary crate by design (no lib.rs), so this bench target includes
//! the source modules directly via `#[path]` instead of changing the binary
//! crate layout.
//!
//! Run with: `cargo bench --bench replay`
//! Quick CI smoke run: `cargo bench --bench replay -- --quick`

// The included source modules expose many TUI functions this bench target
// intentionally does not call, and their `#[cfg(test)]` helper imports are
// irrelevant here.
#![allow(dead_code, unused_imports)]

#[path = "../src/app.rs"]
mod app;
#[path = "../src/buffer.rs"]
mod buffer;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/export.rs"]
mod export;
#[path = "../src/filter.rs"]
mod filter;
#[path = "../src/i18n.rs"]
mod i18n;
#[path = "../src/logcat.rs"]
mod logcat;
#[path = "../src/named_filters.rs"]
mod named_filters;
#[path = "../src/ui.rs"]
mod ui;
#[path = "../src/viewport.rs"]
mod viewport;

use std::hint::black_box;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use regex::Regex;

use app::DisplayConfig;
use buffer::LogBuffer;
use config::ColorPalette;
use logcat::{LogEntry, LogLevel};

fn make_entry(i: u32) -> LogEntry {
    LogEntry {
        timestamp: NaiveDateTime::parse_from_str(
            "2026-08-16 12:00:00.000",
            "%Y-%m-%d %H:%M:%S%.3f",
        )
        .expect("fixed timestamp"),
        pid: 1234,
        tid: 5678,
        level: if i.is_multiple_of(10) {
            LogLevel::Error
        } else {
            LogLevel::Info
        },
        tag: "BenchmarkTag".to_string(),
        message: format!("benchmark message number {i}"),
        package: Some("com.example.bench".to_string()),
    }
}

fn fill_buffer(len: usize) -> LogBuffer {
    let mut buffer = LogBuffer::new();
    for i in 0..len {
        buffer.push(make_entry(i as u32), None);
    }
    buffer
}

struct Sample {
    min: Duration,
    median: Duration,
    max: Duration,
}

fn measure(name: &str, iterations: usize, mut run: impl FnMut()) -> Sample {
    // Warmup before timing.
    run();
    let mut times = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        run();
        times.push(start.elapsed());
    }
    times.sort_unstable();
    let sample = Sample {
        min: times[0],
        median: times[times.len() / 2],
        max: times[times.len() - 1],
    };
    println!(
        "{name:<34} {iterations:>2} iters   min {:>10.2?}   median {:>10.2?}   max {:>10.2?}",
        sample.min, sample.median, sample.max
    );
    sample
}

fn main() {
    let quick = std::env::args().any(|arg| arg == "--quick");
    let iterations = if quick { 1 } else { 3 };

    println!(
        "tlog replay benchmarks ({} mode)",
        if quick { "quick" } else { "normal" }
    );
    println!("- buffer capacity: 100k, eviction batch: 20k");

    measure("buffer_push_100k_no_eviction", iterations, || {
        let buffer = fill_buffer(100_000);
        assert_eq!(buffer.len(), 100_000);
        assert_eq!(buffer.filtered_len(), 100_000);
        black_box(&buffer);
    });

    measure("buffer_push_120k_with_eviction", iterations, || {
        let buffer = fill_buffer(120_000);
        assert!(buffer.total_dropped > 0, "eviction must kick in");
        assert!(buffer.len() <= 100_000);
        black_box(&buffer);
    });

    let mut filtered_buffer = fill_buffer(100_000);
    let level_filter =
        filter::compile("level:WARN", &i18n::EN).expect("benchmark filter must compile");
    measure("filter_rebuild_100k_level", iterations, || {
        filtered_buffer.rebuild_filtered(Some(&level_filter));
        assert_eq!(filtered_buffer.filtered_len(), 10_000);
        black_box(filtered_buffer.filtered_len());
    });

    let display = DisplayConfig::default();
    let palette = ColorPalette::default();
    measure("replay_format_10k_plain", iterations, || {
        let total = filtered_buffer.filtered_len();
        let start = total.saturating_sub(10_000);
        let mut lines = Vec::with_capacity(10_000);
        for pos in start..total {
            if let Some((_, entry)) = filtered_buffer.get_filtered(pos) {
                lines.push(ui::format_entry(entry, &display, &palette, None));
            }
        }
        assert_eq!(lines.len(), 10_000);
        black_box(&lines);
    });

    let search = Regex::new("(?i)benchmark").expect("search regex must compile");
    measure("replay_format_10k_search_highlight", iterations, || {
        let total = filtered_buffer.filtered_len();
        let start = total.saturating_sub(10_000);
        let mut lines = Vec::with_capacity(10_000);
        for pos in start..total {
            if let Some((_, entry)) = filtered_buffer.get_filtered(pos) {
                lines.push(ui::format_entry(entry, &display, &palette, Some(&search)));
            }
        }
        assert_eq!(lines.len(), 10_000);
        black_box(&lines);
    });

    println!("all benchmark scenarios passed their invariants");
}
