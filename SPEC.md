# fu Rust Terminal Viewer MVP Specification

## 1. Product summary

**Owner:** single macOS user working in Ghostty and tmux.

**Problem:** The Bash `fu` viewer depends on ImageMagick and Chafa subprocesses. That creates a visible render delay, made background rendering unsafe because Chafa can probe or read the controlling terminal, and produced incorrect placement through indirect protocol rendering.

**Goal:** Replace Bash `fu` with one Rust binary that displays one screenshot/document image in a Ghostty tmux pane, supports keyboard zoom and pan, and remains responsive during bursts of input.

**Non-goal:** Build a generic terminal image browser, a PDF reader, a GUI image viewer, or a multi-terminal graphics abstraction.

## 2. User stories

1. As a typist reading a long screenshot beside an editor, I run `fu screenshot.png` in a tmux pane and see the top-left image viewport rendered as pixels.
2. I press `j`, `k`, `h`, `l`, page keys, or arrows to move the viewport. A rapid key burst displays the final requested position, not intermediate stale positions.
3. I press `+` or `-` to zoom around the current viewport centre; `0` restores the initial top-left, fit-width view.
4. When tmux blocks graphics, I receive a precise configuration error rather than a blank pane, fallback symbols, or a frozen viewer.
5. On `q`, bare Escape, SIGINT, SIGTERM, or terminal resize, the viewer restores the terminal and removes its graphic placement.

## 3. Success criteria

### Functional

- `fu IMAGE` accepts a readable PNG, JPEG, WebP, GIF, BMP, or TIFF image. For GIF, render the first frame only.
- The initial viewport is anchored at source coordinate `(0, 0)`, fills the pane width, and shows the upper image region. It is never auto-centred.
- All keyboard controls in §6 work in a Ghostty tmux pane with pixel graphics enabled.
- `q`, bare Escape, EOF, SIGINT, and SIGTERM restore terminal mode and remove the displayed Kitty image.
- SIGWINCH remeasures the active pane and redraws without invalid source coordinates.
- Startup rejects unreadable, unsupported, corrupt, or zero-dimension images with a nonzero exit and a one-line `fu:` diagnostic.

### Responsiveness

- Decode the source image exactly once per process.
- The input/event loop never waits for crop, resize, PNG encoding, Base64 encoding, or Kitty transmission.
- During a key burst, render requests are latest-value-wins: only the most recent viewport may be placed after input becomes idle.
- For a 30 MB PNG screenshot, once key input has been idle for 100 ms, the viewer must either display the latest completed render or be actively rendering that same latest viewport. It must not display a known-stale viewport, lose the key, or block input.
- No background task may write to or read from the controlling terminal.

### Visual correctness

- Initial placement begins at terminal cell `(1, 1)` and occupies all columns/rows except the final `STATUS_COLS` (1) column and `STATUS_ROWS` (2) rows.
- The raster is resized to the exact pixel dimensions of its placement. The source crop aspect ratio is derived from real terminal cell pixels, so the image is neither shifted nor distorted.
- The status area is the only text overlay, laid out like a conventional scrollbar: a vertical position bar in the reserved right column (one character per row, alongside the image), a horizontal position bar in the reserved bottom row (under the image, not the corner column), then a plain-text line reporting `x`, `y`, `zoom`, and a rendering indicator. The bottom-right corner cell is left blank. Each bar marks where the current crop sits within the full source extent on that axis.

## 4. Explicit product decisions

| Decision | Requirement |
| --- | --- |
| Primary terminal | Ghostty implementing Kitty Graphics Protocol |
| tmux | Pixel graphics are required, not a symbols fallback |
| tmux prerequisite | `set -g allow-passthrough on` is required; `fu` validates it before entering raw mode |
| Controls | Keyboard only; no mouse, trackpad, or gestures |
| Concurrency | One terminal-owning UI thread and one worker preparing the one-time source upload |
| Cutover | Replace the Bash script cleanly; installed binary remains named `fu` |
| Source support | One still image; animated GIF uses frame zero; PDF, HEIC, SVG, video, directories, and URLs are unsupported |
| Network | No network access at runtime |

## 5. Command-line interface

```text
usage: fu [-z ZOOM] [-o OVERLAP] IMAGE

  -z ZOOM     initial zoom multiplier; finite number >= 1 (default: 1)
  -o OVERLAP  repeated fraction for paging; 0 <= value < 1 (default: 0.12)
  -h, --help  print help and exit
```

- `IMAGE` is exactly one positional path.
- `-z` may be a decimal. `-o` is a decimal fraction only; do not preserve the old percent shorthand.
- The Bash-only `-f` and `-P` options are removed. Supplying either is a usage error with exit status `2`.
- `fu --help` exits `0`; invalid CLI syntax exits `2`.

## 6. Keyboard contract

| Input | Action |
| --- | --- |
| `space`, Enter, `J`, Ctrl-F, PageDown | page down by `viewport_height × (1 - overlap)` |
| `b`, `K`, Ctrl-B, PageUp | page up by the same amount |
| `j`, Down | nudge down by `viewport_height / 8` |
| `k`, Up | nudge up by `viewport_height / 8` |
| `h`, Left | pan left by `viewport_width / 2` |
| `l`, Right | pan right by `viewport_width / 2` |
| `H` | pan left by one full viewport width |
| `L` | pan right by one full viewport width |
| `g` | move to top |
| `G` | move to bottom |
| `+`, `=`, `i` | increment zoom by `1` |
| `-`, `_`, `o` | decrement zoom by `1`; from zoom `1`, fit the whole image |
| `f` | fit the whole image within the draw area, limited by width or height |
| `0` | reset to width fit and source origin `(0, 0)` |
| `r` | retransmit the source and render the current viewport |
| `q`, Escape | quit successfully |

Unknown keys do nothing. Any pending keys already received by the OS are
drained and folded into state before the next placement update.

## 7. Viewport and geometry model

### 7.1 Terminal dimensions

1. Read rows, columns, pixel width, and pixel height with `TIOCGWINSZ` from `/dev/tty`.
2. If `ws_xpixel` or `ws_ypixel` is zero, query cell and window pixel dimensions using Kitty-compatible `CSI 16 t` and `CSI 14 t` before enabling raw mode.
3. If physical dimensions remain unavailable or inconsistent, exit with `fu: terminal did not report pixel dimensions`; do not invent a cell ratio.
4. Reserve `STATUS_COLS` (1) terminal column for a vertical position bar and `STATUS_ROWS` (2) terminal rows for a horizontal position bar plus one text line. The draw area is `max(columns - STATUS_COLS, 1) × max(rows - STATUS_ROWS, 1)` cells.

### 7.2 Source coordinates

- State is `View { zoom: f32, x: u32, y: u32, fit: FitMode }` plus current `Geometry`.
- In `FitMode::Width` at `zoom = 1`, crop width is the entire source width. Crop height is `crop_width × draw_pixel_height / draw_pixel_width`, clamped to source height. Initial `x = 0`, `y = 0`.
- In `FitMode::Width` at `zoom > 1`, crop width is `source_width / zoom`; crop height follows the same terminal pixel aspect ratio.
- In `FitMode::Whole`, the crop is the complete source image. The Kitty placement is constrained by whichever of draw width or height preserves the image aspect ratio within the pane.
- Clamp `x` to `[0, source_width - crop_width]` and `y` to `[0, source_height - crop_height]` after every input, resize, or zoom operation.
- Zoom preserves the old viewport centre where that centre remains legal; otherwise clamp to the nearest legal crop. Zooming in from whole-image fit returns to width fit at zoom `1`.
- On terminal resize, preserve the source-space viewport centre in width-fit mode; whole-image fit remains anchored at `(0, 0)`.

### 7.3 Rendering pipeline

1. Main thread decodes the file through `image` into one `Arc<RgbaImage>`.
2. Before raw mode, main thread creates `ratatui_image::picker::Picker` with `Picker::from_query_stdio()` and requires its selected protocol to be Kitty.
3. A worker PNG-compresses and base64-encodes the decoded source into one chunked Kitty upload. It never reads terminal input or writes terminal output.
4. Main thread sends that upload once and creates one identified placement.
5. Pan, scroll, zoom, fit, and resize replace the same placement with `x`, `y`, `w`, `h`, and one limiting `c` or `r` value. No navigation action crops, resizes, encodes, or retransmits pixel data.
6. Main thread alone calls `Terminal::draw`, emits Kitty commands, writes the status UI, and performs identified-image cleanup.

## 8. ratatui-image integration contract

### 8.1 Capability gate

- When `$TMUX` is unset, `Picker::from_query_stdio()` must select `ProtocolType::Kitty`.
- When `$TMUX` is set, execute `tmux show-option -gqv allow-passthrough` before terminal probing.
  - Output `on`: initialize the picker and require `ProtocolType::Kitty`. `fu` wraps upload, placement, and cleanup commands in tmux DCS passthrough.
  - Any other output, command failure, nonzero status, or a non-Kitty protocol: exit `1` with exactly:

```text
fu: tmux pixel graphics require: set -g allow-passthrough on
```

- The MVP does not auto-edit tmux configuration and does not attempt character-cell fallback.

### 8.2 Library ownership

- Configure `ratatui-image` with `default-features = false` and features `["image-defaults", "crossterm"]`; this excludes its optional Chafa integration.
- `ratatui-image` owns protocol probing and physical cell-pixel measurement.
- `fu` owns one Kitty RGBA upload, one identified source-rectangle placement, tmux DCS wrapping, and identified-image cleanup. This narrow transport is required so navigation can reuse terminal-resident pixels; `ratatui-image` 11.0.6 creates a new encoded image for every changed source crop.
- Pin `ratatui-image` to `11.0.6`; upgrade only after the focused PTY protocol tests and manual Ghostty/tmux acceptance flow pass.

The integration retains `ratatui-image`'s terminal capability and font-size
probing while using the Kitty protocol's native reusable placements for
constant-size navigation updates.

## 9. Architecture and project structure

```text
Cargo.toml                  package metadata and pinned Rust dependencies
Cargo.lock                  committed dependency lockfile
src/main.rs                 CLI, lifecycle, error reporting, process exit
src/app.rs                  Ratatui event loop, raw terminal guard, state transitions
src/input.rs                terminal event decoding and key-to-command mapping
src/view.rs                 geometry, crop calculation, pan/zoom/reset rules
src/render.rs               one-time Kitty upload and reusable source-rectangle placement
src/protocol.rs             Picker setup, Kitty requirement, tmux policy adapter
src/terminal.rs             tmux preflight, status, terminal restore
tests/cli.rs                command-line and failure-path integration tests
tests/pty.rs                pseudo-terminal lifecycle, tmux, and input-burst tests
tests/protocol.rs           Picker and tmux policy tests
devdocs/fyi.md              decisions and implementation outcomes
README.md                   installation, tmux prerequisite, usage, controls
```

### Core interfaces

```rust
pub struct Geometry {
    pub columns: u16,
    pub rows: u16,
    pub draw_rows: u16,
    pub pixel_width: u32,
    pub pixel_height: u32,
}

pub enum FitMode {
    Width,
    Whole,
}

pub struct View {
    pub zoom: f32,
    pub x: u32,
    pub y: u32,
    pub fit: FitMode,
}

pub struct KittyImage {
    // one image id, one retained upload, and transmission state
}
```

## 10. Technology stack

| Component | Version | Purpose |
| --- | --- | --- |
| Rust | `1.97.0` minimum | language and toolchain observed in this repository |
| `ratatui` | `0.30.2` | immediate-mode TUI frame and terminal lifecycle |
| `ratatui-image` | `11.0.6` | Kitty capability probing and physical cell-size measurement |
| `image` | `0.25.10` | source decode |
| `crossterm` | `0.29.0` | raw mode, keyboard events, resize events, terminal cleanup |
| `clap` | `4.6.5` | CLI parsing and usage errors |
| `thiserror` | `2.0.19` | typed user-facing errors |
| `base64` | `0.22.1` | one-time PNG payload encoding |

Do not introduce ImageMagick, Chafa, a shell subprocess renderer, a second terminal graphics library, an async runtime, or a cache directory.

## 11. Error handling and cleanup

- Use a `TerminalGuard` created only after the tmux gate and image decode succeed. Its `Drop` implementation restores cursor visibility, raw mode, alternate screen state if used, and graphics cleanup.
- Install signal handling such that SIGINT and SIGTERM request normal event-loop shutdown; terminal writes remain owned by the main thread.
- Never discard an I/O, image decode, terminal query, or tmux command error silently.
- Status must show `rendering…` while the one-time source upload is being prepared; errors leave raw mode and print one diagnostic to stderr.
- If the renderer channel closes unexpectedly, exit nonzero with `fu: renderer stopped unexpectedly`.

## 12. Testing and verification

### Automated tests

Run all commands from repository root:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

Required coverage:

1. `view.rs` unit tests: initial top-left crop, full-image fit, all clamps, page overlap, zoom-centre preservation, reset, and resize centre preservation.
2. `input.rs` unit tests: every documented keyboard mapping, raw Escape, incomplete escape sequences, and unknown input.
3. `render.rs` tests: one-time source transmission, payload-free subsequent placement updates, whole-image height limiting, tmux wrapping, and off-thread upload preparation.
4. `protocol.rs` tests: tmux-off fails before `Picker` initialization and non-Kitty picker selection fails.
5. `tests/cli.rs`: help, missing image, invalid options, corrupt image, unsupported image, and tmux passthrough-disabled error.
6. `tests/pty.rs`: raw mode restoration after `q`, SIGINT, and SIGTERM; input bursts exit cleanly; scrolling emits a replacement placement without pixel transmission; whole-image fit places the complete portrait source against pane height.

### Manual acceptance run

In Ghostty, configure tmux once:

```tmux
set -g allow-passthrough on
```

Then run:

```sh
cargo build --release
./target/release/fu /absolute/path/to/30mb-screenshot.png
```

Verify in a tmux pane:

- first placement begins at the top-left with no unexpected horizontal offset;
- all required keys behave as §6 states;
- a sustained `hjkl` burst remains responsive and settles on the final viewport;
- resizing the pane does not misplace or distort the image;
- `q`, Ctrl-C, and closing the pane restore cursor and terminal input;
- `tmux set -g allow-passthrough off` produces the exact configuration error and no blank raw-mode pane.

## 13. Boundaries

### ✅ Always

- Keep source image data and terminal output ownership in-process.
- Use physical terminal pixel dimensions; fail rather than guessing them.
- Keep all UI mutation and terminal I/O on the main thread.
- Run formatting, Clippy, tests, release build, and manual Ghostty/tmux acceptance before committing.
- Record material decisions and verification in `devdocs/fyi.md`.

### ⚠️ Ask first

- Add a dependency beyond §10.
- Add support for another terminal protocol, mouse input, another image format, or PDF/HEIC/SVG/video.
- Change documented keyboard controls or CLI flags.
- Change tmux configuration automatically.
- Change performance targets or install a global binary.

### 🚫 Never

- Spawn Chafa, ImageMagick, `timg`, or another renderer from the application.
- Let a worker thread read stdin, write stdout/stderr, open `/dev/tty`, or emit terminal escape sequences.
- Use a fabricated cell aspect fallback when pixel dimensions are absent.
- Keep the Bash implementation, alias, or compatibility shim after Rust cutover.
- Commit secrets, modify CI, or add network access.

## 14. Migration

1. Build the Rust application and automated test suite alongside the existing Bash script.
2. Complete the manual Ghostty/tmux acceptance run against the real 30 MB screenshot.
3. Replace root `fu` Bash script with Cargo package sources and installable `fu` binary workflow.
4. Delete `tests/test_fu.py`, Bash-only README content, `-f`, and `-P` documentation in the same change.
5. Update README installation to:

```sh
cargo install --path .
```

6. Update `devdocs/fyi.md` with the clean-cutover decision, tmux requirement, and benchmark/acceptance evidence.

## 15. Agent-sized implementation tasks

### Task 1 — Scaffold Rust package and model view state

Create `Cargo.toml`, `src/main.rs`, `src/view.rs`, and `tests/cli.rs`. Implement CLI validation plus pure viewport/geometry functions and tests. Do not alter terminal state or remove Bash yet.

**Acceptance:** `cargo test view` passes; invalid CLI behavior is tested; no terminal protocol code exists.

### Task 2 — Implement terminal lifecycle and keyboard input

Create `src/terminal.rs`, `src/input.rs`, and `src/app.rs`. Add `TerminalGuard`, pixel-size querying, tmux passthrough preflight, signal-driven shutdown, and pure command application. Use a fake writer in tests.

**Acceptance:** PTY tests prove raw mode restoration and every key mapping; tmux-off exits with the exact error before raw mode.

### Task 3 — Integrate Kitty capability probing

Create `src/protocol.rs` and `tests/protocol.rs`. Initialize
`Picker::from_query_stdio()`, enforce Kitty selection, and connect tmux
preflight to capability detection.

**Acceptance:** non-Kitty selection and disabled tmux passthrough fail before raw mode.

### Task 4 — Implement terminal-resident viewport rendering

Create `src/render.rs`. Prepare one source upload off-thread, then reuse one
identified Kitty placement for every source crop.

**Acceptance:** deterministic tests demonstrate that navigation emits only a
small placement update and never retransmits source pixels.

### Task 5 — Integrate, validate, and cut over

Wire app, input, renderer, and the ratatui-image widget. Add PTY burst tests, run the manual Ghostty/tmux acceptance flow, replace the Bash script and tests, update README and `devdocs/fyi.md`.

**Acceptance:** every command in §12 passes; all manual acceptance points pass; no Bash or external-renderer dependency remains.
