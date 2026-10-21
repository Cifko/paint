# Paint

This is simple paint tool, not a photoshop. The main goal is to provide a tool that's fast to load, easy to use.

Written in Rust with [egui/eframe](https://github.com/emilk/egui); runs on Windows, Linux and macOS.

## Features

- **Infinite canvas** – paint anywhere; pan and zoom freely. 100% zoom is pixel-for-pixel
  with the screen, also on displays with OS scaling (e.g. 150% on Windows).
- **Tools** – Brush, Eraser, Flood fill, Rectangle and Circle (outline or filled),
  Grid, rectangle Select.
- **Copy / paste** – copy a selected rectangle, paste images from the system clipboard
  (e.g. a screenshot from the Snipping Tool), then move the pasted piece into place.
  White is treated as transparent when pasting (toggleable).
- **Color picker** in the top bar plus a quick palette.
- **Floating size and opacity sliders** on the left side of the canvas.
- **Quick brush resize** – hold Alt (Option on macOS) and drag left/right or scroll;
  the key and behavior are configurable in **Settings**.
- **Layers** – previews, add, delete, reorder, hide, rename, per-layer opacity.
- **Undo / redo** for painting and layer add/delete/reorder.

## Controls

| Action | Input |
| --- | --- |
| Paint / use tool | Left mouse button |
| Pan | Middle mouse drag, or Space + left drag |
| Zoom | Mouse wheel / pinch (around the cursor) |
| Brush / Eraser / Fill | `B` / `E` / `G` |
| Rectangle / Circle / Grid / Select | `R` / `C` / `T` / `M` |
| Snap rectangle, cell, grid or selection to a square | Hold `Shift` while drawing (configurable) |
| Change brush size | Hold `Alt` (`Option` on macOS) + drag left/right or scroll (configurable) |
| Copy selection (active layer) | `Ctrl+C` |
| Copy selection (all visible layers) | `Ctrl+Shift+C` |
| Cut / delete selection | `Ctrl+X` / `Delete` |
| Paste image from clipboard | `Ctrl+V` |
| Place / cancel pasted image | `Enter` or click outside it / `Esc` |
| Undo | `Ctrl+Z` (`Cmd+Z` on macOS) |
| Redo | `Ctrl+Shift+Z` or `Ctrl+Y` |

Circle: press at the center and drag out the radius. Rectangle: drag from corner to
corner. For both, the pen size is the outline width.

Grid: click where the grid starts, move the mouse to size one cell, click, then move
to size the whole grid and click again. As many whole cells as fit are drawn (e.g. a
3×7 cell in a 10×20 area gives 3×2 cells); the count is shown while you move.
`Esc` or right-click cancels.

Pasted images float above the active layer until placed; drag them to move.

Flood fill works on the active layer. Because the canvas is infinite, a fill
is limited to the visible area plus the area the layer already has paint in.

## Build & run

Requires Rust 1.95 or newer ([rustup](https://rustup.rs)).

```sh
cargo run --release
```

On Linux you may need the usual windowing libraries, e.g. on Debian/Ubuntu:

```sh
sudo apt install libxkbcommon-x11-0 libx11-6 libxcursor1 libxrandr2 libxi6 libwayland-client0
```

## Development

```sh
cargo test                         # unit tests (canvas, fill, undo/redo)
cargo clippy --all-targets -- -D warnings
cargo fmt
```

Code layout:

- `src/main.rs` – window setup.
- `src/app.rs` – UI (toolbar, floating panels, canvas input and rendering).
- `src/canvas.rs` – tiled infinite layers, brush/circle rasterization, flood fill,
  copy/paste and thumbnails.
- `src/history.rs` – undo/redo.
- `src/settings.rs` – user preferences (saved between runs).
