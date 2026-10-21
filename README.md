# Paint

This is simple paint tool, not a photoshop. The main goal is to provide a tool that's fast to load, easy to use.

Written in Rust with [egui/eframe](https://github.com/emilk/egui); runs on Windows, Linux and macOS.

## Features

- **Infinite canvas** – paint anywhere; pan and zoom freely. 100% zoom is pixel-for-pixel
  with the screen, also on displays with OS scaling (e.g. 150% on Windows).
- **Open / save** PNG, JPEG and BMP (File menu, `Ctrl+O` / `Ctrl+S`, drag a file onto
  the window, or `paint image.png`). Saving flattens the visible layers and crops to
  what's drawn; PNG keeps transparency, JPEG/BMP get a white background.
- **Tools** – Brush, Eraser, Flood fill, Rectangle and Ellipse (outline or filled),
  Grid, rectangle Select, Text.
- **Copy / paste** – copy a selected rectangle, paste images from the system clipboard
  (e.g. a screenshot from the Snipping Tool), then move the pasted piece into place.
  Color 2 (white by default) is treated as transparent when pasting (toggleable).
- **Two colors, like MS Paint** – the left mouse button paints with color 1, the right
  button with color 2. Left-click a palette swatch to set color 1, right-click to set
  color 2; click either color in the toolbar for a full picker, `X` swaps them. The last
  swatch is **Clear**: with it, Fill (and Brush, shapes, Text) erase instead of painting.
- **Floating size and opacity sliders** on the left side of the canvas. Painting tools
  share one size/opacity; the eraser and the text tool each keep their own.
- **Quick brush resize** – hold Alt (Option on macOS) and drag left/right or scroll;
  the key and behavior are configurable in **Settings**.
- **Layers** – previews, add, delete, reorder, hide, rename, per-layer opacity.
- **Undo / redo** for painting and layer add/delete/reorder.

## Controls

| Action | Input |
| --- | --- |
| Paint / use tool | Left mouse button (color 1), right mouse button (color 2) |
| Swap color 1 and 2 | `X` |
| New / open / save / save as | `Ctrl+N` / `Ctrl+O` / `Ctrl+S` / `Ctrl+Shift+S` |
| Pan | Middle mouse drag, or Space + left drag |
| Zoom | Mouse wheel / pinch (around the cursor) |
| Brush / Eraser / Fill | `B` / `E` / `G` |
| Rectangle / Ellipse / Grid / Select / Text | `R` / `C` / `H` / `M` / `T` |
| Snap rectangle, ellipse, cell, grid or selection to a square / circle | Hold `Shift` while drawing (configurable) |
| Select everything that's painted | `Ctrl+A` |
| Change brush size | Hold `Alt` (`Option` on macOS) + drag left/right or scroll (configurable) |
| Copy selection (active layer) | `Ctrl+C` |
| Copy selection (all visible layers) | `Ctrl+Shift+C` |
| Cut / delete selection | `Ctrl+X` / `Delete` |
| Paste image from clipboard | `Ctrl+V` |
| Place / cancel pasted image | `Enter` or click outside it / `Esc` |
| Undo | `Ctrl+Z` (`Cmd+Z` on macOS) |
| Redo | `Ctrl+Shift+Z` or `Ctrl+Y` |

Text: click where the text should start and type, like in a text editor: there's no
box, `Enter` starts a new line, arrows/Home/End move the caret. `Esc`, clicking
elsewhere or switching tools places the text; `Ctrl+Z` while typing discards it. The
size slider sets the font size; the font is chosen in the side panel (DejaVu fonts,
bundled).

Rectangle and Ellipse: drag from corner to corner of the shape's bounding box. The pen
size is the outline width.

The selection belongs to the Select tool: it's only shown (and only used by copy, cut
and delete) while that tool is active.

Grid: pick the cell shape (rectangles, triangles or hexagons) in the toolbar next to
the tools, click where the grid starts, move the mouse to size one cell, click, then move
to size the whole grid and click again. As many whole cells as fit are drawn (e.g. a
3×7 cell in a 10×20 area gives 3×2 cells); the count is shown while you move. With
`Shift` the cell becomes regular (square, equilateral triangle, regular hexagon).
`Esc` cancels.

Pasted images float above the active layer until placed; drag them to move.

Flood fill works on the active layer. Filling an enclosed area paints it. Filling an
area that is open to infinity (e.g. around a drawn shape) changes the layer's
background color everywhere, out to infinity, while enclosed areas keep their look:
draw a circle, fill outside it with red, and the inside stays white. Undo restores
the previous background. Filling with the Clear color erases the area instead; on
the open area it makes the infinite background clear again.

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
- `src/file.rs` – loading and saving images.
- `src/text.rs` – text tool fonts, layout and editing.
- `src/settings.rs` – user preferences (saved between runs).
