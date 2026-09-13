# fu

Pan-and-zoom viewer for one screenshot/document image in a Ghostty tmux
pane. Built for typing practice: keep a screenshot of text in one terminal
split, your editor in the other, and press `space` to advance through it.

`fu` is a single Rust binary. It decodes and uploads the image once, then
moves and scales one Kitty placement by source coordinates. Scrolling,
panning, and zooming send only a small placement command instead of
reprocessing or retransmitting image pixels.

## Requirements

- **Ghostty**, or another terminal implementing the Kitty graphics protocol.
- **tmux**, if used, needs pixel graphics enabled:

  ```tmux
  set -g allow-passthrough on
  ```

  `fu` checks this before doing anything else and refuses to start with a
  clear error if it's off. There is no symbols/block-character fallback.

## Install

Install the latest version directly from GitHub:

```sh
cargo install --git https://github.com/rfist/fu
```

Or build from a checkout:

```sh
git clone https://github.com/rfist/fu.git
cd fu
cargo install --path .
```

## Usage

```sh
fu shot.png              # fit the image width to the pane
fu -z 3 shot.png         # start zoomed in (crop is 1/3 the source width)
fu -o 0.25 shot.png      # repeat 25% of the view when paging
```

```text
usage: fu [-z ZOOM] [-o OVERLAP] IMAGE

  -z ZOOM     initial zoom multiplier; finite number >= 1 (default: 1)
  -o OVERLAP  repeated fraction for paging; 0 <= value < 1 (default: 0.12)
  -h, --help  print help and exit
```

Supported formats: PNG, JPEG, WebP, GIF (first frame only), BMP, TIFF.

| key | action |
| --- | --- |
| `space` `Enter` `J` `^F` `PageDown` | page down (with overlap) |
| `b` `K` `^B` `PageUp` | page up (with overlap) |
| `j` / `Down` | nudge down |
| `k` / `Up` | nudge up |
| `h` / `Left` | pan left |
| `l` / `Right` | pan right |
| `H` / `L` | pan left/right a full viewport width |
| `g` / `G` | jump to top / bottom |
| `i` `+` `=` / `o` `-` `_` | zoom in / out; zooming out past `1` fits the whole image |
| `f` | fit the whole image inside the pane |
| `0` | reset to width fit at the top-left |
| `r` | force a fresh upload and render |
| `q` / `Esc` | quit |

`q`, Escape, Ctrl-C, and closing the pane all restore the terminal and remove
the displayed image.

## How it works

The initial viewport is anchored at the image's top-left corner and fills the
pane width; paging keeps a small overlap so you never lose your place
mid-line. Press `f`, or zoom out once past `1`, to fit the complete image by
whichever of its width or height is limiting. Crop dimensions are derived from
the pane's real per-cell pixel size (via `ratatui-image`'s terminal probing),
not an assumed character-cell aspect ratio.

Decoding happens once, on the main thread. A background worker prepares one
Kitty upload before the source pixels are released. The terminal then keeps
that source image: every pan, scroll, zoom, or resize replaces the same
placement with new source coordinates and transfers no pixel payload.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

See `SPEC.md` for the full product/technical specification and
`devdocs/fyi.md` for implementation decisions.

## License

MIT
