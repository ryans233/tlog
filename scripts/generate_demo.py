#!/usr/bin/env python3
"""Generate assets/demo.gif, an illustrative tlog demo animation.

The frames are drawn with Pillow and mirror tlog's real log formatting
(timestamp/PID/TID/level/tag colors and the bottom filter bar). Regenerate
with:

    python scripts/generate_demo.py
"""

from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

WIDTH = 800
HEIGHT = 460
ROWS = 18
ROW_H = 20
LOG_TOP = 0
LOG_BOTTOM = ROWS * ROW_H
BAR_TOP = LOG_BOTTOM

BG = (17, 24, 28)
BAR_BG = (30, 34, 38)
BORDER = (64, 64, 64)
CURSOR = (0, 183, 183)
TEXT = (220, 220, 220)
DIM = (128, 128, 128)
TAG = (0, 183, 183)
TIMESTAMP = (128, 128, 128)
LEVEL_COLORS = {
    "V": (128, 128, 128),
    "D": (0, 183, 183),
    "I": (0, 200, 0),
    "W": (220, 200, 0),
    "E": (255, 70, 70),
    "F": (255, 70, 70),
}
RUNNING = (0, 200, 0)

FONT_PATH = Path("C:/Windows/Fonts/consola.ttf")
FALLBACK_FONT_PATH = Path("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf")


def load_font(size: int) -> ImageFont.FreeTypeFont:
    path = FONT_PATH if FONT_PATH.exists() else FALLBACK_FONT_PATH
    return ImageFont.truetype(str(path), size)


DEMO_LINES = [
    ("D", "tlog", "demo feed started (illustrative animation)"),
    ("I", "ActivityManager", "Start proc 1234:com.example.demo for activity .MainActivity"),
    ("I", "tlog", "Press / to search, v d i w e for quick levels, E to export"),
    ("D", "OpenGLRenderer", "setSurfaceFormat called with format 1"),
    ("I", "MainActivity", "onCreate completed in 42 ms"),
    ("W", "InputMethodManager", "Slow operation: 125 ms in measure()"),
    ("E", "MainActivity", "failed to fetch profile: network timeout"),
    ("I", "tlog", "tab into the filter input and try tag:MainActivity"),
    ("V", "Surface", "surfaceCreated"),
    ("D", "NetworkClient", "GET /api/status -> 200 (18 ms)"),
    ("W", "BatteryManager", "level dropped to 15%"),
    ("E", "FATAL EXCEPTION", "Process: com.example.demo, PID: 1234"),
    ("E", "AndroidRuntime", "java.lang.IllegalStateException: boom"),
    ("I", "tlog", "color presets and display options live in Settings (o)"),
    ("I", "ActivityManager", "Process com.example.demo (pid 1234) has died"),
    ("W", "tlog", "press p to pause, g to bypass the active filter"),
    ("D", "NetworkClient", "retry scheduled in 2s"),
    ("I", "MainActivity", "onResume called"),
    ("V", "Choreographer", "Skipped 2 frames"),
    ("I", "tlog", "exported logs replay cleanly with: tlog --file log.txt"),
    ("W", "PackageManager", "Unknown permission in manifest"),
    ("D", "OpenGLRenderer", "eglCreateContext succeeded"),
    ("E", "CrashReporter", "uncaught exception captured"),
    ("I", "MainActivity", "screen rotation handled"),
    ("I", "tlog", "arrow keys edit filters with UTF-8 safe cursor movement"),
    ("V", "Surface", "surfaceChanged 1080x2400"),
    ("D", "NetworkClient", "POST /api/events -> 204 (31 ms)"),
    ("W", "AudioTrack", "underrun observed"),
    ("I", "ActivityManager", "Start proc 5678:com.example.background for service"),
    ("E", "CrashReporter", "uploading crash report"),
    ("I", "tlog", "quick levels combine with AND: v then e keeps level:E"),
    ("D", "NetworkClient", "connection pool idle"),
    ("I", "MainActivity", "onStop called"),
    ("W", "tlog", "paused: incoming lines are discarded by design"),
    ("E", "AndroidRuntime", "FATAL EXCEPTION: main"),
    ("I", "tlog", "press Ctrl+S to save the current filter by name"),
    ("V", "Choreographer", "frame time 16.7 ms"),
    ("D", "OpenGLRenderer", "destroying context"),
    ("I", "MainActivity", "onDestroy completed"),
    ("I", "tlog", "offline playback exits at EOF -- handy for CI demos"),
]

STATUSES = [
    "20 total | 16 shown | 4 filtered out | 0 dropped | Filter OK",
    "28 total | 19 shown | 9 filtered out | 0 dropped | Filter OK",
    "34 total | 24 shown | 10 filtered out | 0 dropped | Filter OK",
    "40 total | 28 shown | 12 filtered out | 0 dropped | Filter OK",
    "40 total | 28 shown | 12 filtered out | 0 dropped | Filter OK",
]


def draw_log_line(draw: ImageDraw.ImageDraw, y: int, level: str, tag: str, msg: str, font) -> None:
    ts = "08-16 14:03:"
    pid = "  1234"
    tid = "  5678"
    level_color = LEVEL_COLORS[level]
    draw.text((8, y), ts, font=font, fill=TIMESTAMP)
    draw.text((118, y), pid, font=font, fill=DIM)
    draw.text((172, y), tid, font=font, fill=DIM)
    draw.text((226, y), f"{level} ", font=font, fill=level_color)
    draw.text((244, y), f"{tag}: ", font=font, fill=TAG)
    draw.text((244 + draw.textlength(f"{tag}: ", font=font), y), msg, font=font, fill=TEXT)


def draw_bar(img: ImageDraw.ImageDraw, font, small_font, frame: int) -> None:
    img.rectangle((0, BAR_TOP, WIDTH, HEIGHT), fill=BAR_BG)
    img.line((0, BAR_TOP, WIDTH, BAR_TOP), fill=CURSOR, width=1)
    status_index = min(frame // 12, len(STATUSES) - 1)
    title = f" {RUNNING_CHAR} RUNNING   Filter ({STATUSES[status_index]}) "
    img.text((6, BAR_TOP + 4), title, font=small_font, fill=DIM)
    img.rectangle((2, BAR_TOP + 1, 2, HEIGHT - 1), fill=CURSOR)
    img.rectangle((WIDTH - 3, BAR_TOP + 1, WIDTH - 3, HEIGHT - 1), fill=CURSOR)
    img.text((8, BAR_TOP + 26), "> ", font=font, fill=CURSOR)
    img.text((24, BAR_TOP + 26), "tag:MainActivity level:E", font=font, fill=TEXT)
    img.text((24, BAR_TOP + 50), "Filter OK | saved filters: name:errors", font=small_font, fill=DIM)


RUNNING_CHAR = "\u25cf"


def main() -> None:
    font = load_font(15)
    small_font = load_font(13)
    frames = []
    total_frames = 48
    visible: list[tuple[str, str, str]] = []
    for frame in range(total_frames):
        img = Image.new("RGB", (WIDTH, HEIGHT), BG)
        draw = ImageDraw.Draw(img)
        if frame % 2 == 0 and len(visible) < len(DEMO_LINES):
            visible.append(DEMO_LINES[len(visible)])
        shown = visible[-ROWS:] if len(visible) > ROWS else visible
        for row, (level, tag, msg) in enumerate(shown):
            draw_log_line(draw, LOG_TOP + row * ROW_H, level, tag, msg, font)
        draw_bar(draw, font, small_font, frame)
        frames.append(img)

    out = Path(__file__).resolve().parents[1] / "assets" / "demo.gif"
    frames[0].save(
        out,
        save_all=True,
        append_images=frames[1:],
        duration=90,
        loop=0,
        optimize=True,
    )
    print(f"wrote {out} ({out.stat().st_size} bytes, {len(frames)} frames)")


if __name__ == "__main__":
    main()
