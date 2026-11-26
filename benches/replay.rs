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
#[path = "../src/scrollback.rs"]
mod scrollback;
#[path = "../src/ui.rs"]
mod ui;
#[path = "../src/viewport.rs"]
mod viewport;

use std::hint::black_box;
use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use regex::Regex;
use tokio::sync::mpsc;

use app::{App, DisplayConfig, FeedDrain, MSG_CHANNEL_CAPACITY, MSG_DRAIN_BATCH};
use buffer::LogBuffer;
use config::ColorPalette;
use logcat::{LogEntry, LogLevel, Message};

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
        let _ = buffer.push(make_entry(i as u32), None);
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

    // Burst ingest: a log storm delivers far more lines per frame than the UI
    // needs to draw. The pre-batching loop shape dispatched one message, then
    // wrote one scrollback batch and drew one frame for it. The batched shape
    // drains `MSG_DRAIN_BATCH` messages and writes one scrollback batch per
    // drain. Both run the same `App` dispatch and ANSI rendering here, so the
    // delta is the per-line flush/draw work; the actual kernel-side write count
    // drops from one per line to one per batch.
    const BURST: usize = 4096;
    const BURST_VIEWPORT_TOP: u16 = 24;
    println!(
        "- burst ingest: {BURST} entries, channel {MSG_CHANNEL_CAPACITY}, drain batch {MSG_DRAIN_BATCH}"
    );

    let per_line = measure("burst_ingest_per_line", iterations, || {
        let mut state = App::new(i18n::Lang::En);
        let (tx, mut rx) = mpsc::channel::<Message>(MSG_CHANNEL_CAPACITY);
        for i in 0..BURST {
            tx.try_send(Message::NewEntry(make_entry(i as u32)))
                .expect("burst must fit the channel");
        }
        while let Ok(message) = rx.try_recv() {
            state.dispatch(message);
            for line in state.pending_log_lines.drain(..) {
                let bytes =
                    scrollback::insert_batch_ansi(BURST_VIEWPORT_TOP, std::slice::from_ref(&line))
                        .expect("ANSI rendering must not fail");
                black_box(&bytes);
            }
        }
        assert_eq!(state.buffer.len(), BURST);
        assert!(state.pending_log_lines.is_empty());
        black_box(state.buffer.len());
    });

    let batched = measure("burst_ingest_batched", iterations, || {
        let mut state = App::new(i18n::Lang::En);
        let (tx, mut rx) = mpsc::channel::<Message>(MSG_CHANNEL_CAPACITY);
        for i in 0..BURST {
            tx.try_send(Message::NewEntry(make_entry(i as u32)))
                .expect("burst must fit the channel");
        }
        while let Ok(first) = rx.try_recv() {
            if state.drain_feed(&mut rx, Some(first), MSG_DRAIN_BATCH, false, true)
                == FeedDrain::Closed
            {
                break;
            }
            if !state.pending_log_lines.is_empty() {
                let bytes =
                    scrollback::insert_batch_ansi(BURST_VIEWPORT_TOP, &state.pending_log_lines)
                        .expect("ANSI rendering must not fail");
                black_box(&bytes);
                state.pending_log_lines.clear();
            }
        }
        assert_eq!(state.buffer.len(), BURST);
        assert!(state.pending_log_lines.is_empty());
        black_box(state.buffer.len());
    });

    let rate = |sample: &Sample| BURST as f64 / sample.median.as_secs_f64().max(f64::MIN_POSITIVE);

    // One viewport draw per event-loop iteration is the other half of the
    // per-line cost: the pre-batching loop drew a frame for every log line.
    // Rendered into a plain buffer so no TTY is needed; the diff+flush pass is
    // not included, so this understates the real saving.
    const FRAMES: u32 = 1000;
    let draw = measure("viewport_render_1k_frames", iterations, || {
        let state = App::new(i18n::Lang::En);
        let area = ratatui::layout::Rect::new(0, 0, 120, 3);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        for _ in 0..FRAMES {
            let mut frame = viewport::ViewportFrame {
                cursor_position: None,
                viewport_area: area,
                buffer: &mut buffer,
            };
            ui::render_viewport(&mut frame, &state);
        }
        black_box(&buffer);
    });
    let draw_per_frame = draw.median.as_secs_f64() / FRAMES as f64;
    let batches = (BURST as f64 / MSG_DRAIN_BATCH as f64).ceil();
    let per_line_with_draw = 1.0 / (1.0 / rate(&per_line) + draw_per_frame);
    let batched_with_draw = 1.0 / (1.0 / rate(&batched) + draw_per_frame / batches);

    println!(
        "burst ingest ({BURST} entries): per-line {:.0} msg/s, batched {:.0} msg/s ({:.2}x)",
        rate(&per_line),
        rate(&batched),
        rate(&batched) / rate(&per_line)
    );
    println!(
        "with one draw per iteration ({:.1?}/frame): per-line {:.0} msg/s, batched {:.0} msg/s ({:.2}x)",
        draw.median / FRAMES,
        per_line_with_draw,
        batched_with_draw,
        batched_with_draw / per_line_with_draw
    );

    println!("all benchmark scenarios passed their invariants");
}
