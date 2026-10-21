# Paint

This is simple paint tool, not a photoshop. The main goal is to provide a tool that's fast to load, easy to use.

Written in Rust with [egui/eframe](https://github.com/emilk/egui); runs on Windows, Linux and macOS.

## Features

- **Infinite canvas** – paint anywhere; pan and zoom freely.
- **Tools** – Brush, Eraser, Flood fill, Circle (outline or filled).
- **Color picker** in the top bar plus a quick palette.
- **Floating size and opacity sliders** on the left side of the canvas.
- **Layers** – add, delete, reorder, hide, rename, per-layer opacity.
- **Undo / redo** for painting and layer add/delete/reorder.

## Controls

| Action | Input |
| --- | --- |
| Paint / use tool | Left mouse button |
| Pan | Middle mouse drag, or Space + left drag |
| Zoom | Mouse wheel / pinch (around the cursor) |
| Brush / Eraser / Fill / Circle | `B` / `E` / `G` / `C` |
| Undo | `Ctrl+Z` (`Cmd+Z` on macOS) |
| Redo | `Ctrl+Shift+Z` or `Ctrl+Y` |

Circle: press at the center and drag out the radius. The pen size is the outline width.

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
- `src/canvas.rs` – tiled infinite layers, brush/circle rasterization, flood fill.
- `src/history.rs` – undo/redo.
