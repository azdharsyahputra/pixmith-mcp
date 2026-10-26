//! Built-in documentation, also served as MCP resources and as the server instructions.

use super::{Out, READ, ToolDef, tool};
use crate::engine::{Engine, R};
use schemars::JsonSchema;
use serde::Deserialize;

pub const LUA_API: &str = include_str!("../../docs/LUA_API.md");

/// Sent as the server's `instructions` and returned by `help` (topic guide).
pub const GUIDE: &str = r##"# Pixmith MCP: how to use it

Pixmith is a pixel-art editor (sprites, animation, tilemaps; .pxmt and .aseprite files). This server runs its
engine in-process; every tool call edits real documents with the same code the desktop apps use.

## Conventions
- Documents have ids ("d1", "d2", ...) from doc_new / doc_open (in the Pixmith app: "app:<id>", see Live mode).
  `doc` may be omitted when only one is open (with the app connected: the app's active document).
- Files: with --root every path must be inside it; --allow-dir folders (e.g. a temp dir) additionally take reads and export output.
- Pixel coordinates are 0-based from the top-left. Rectangles are {x, y, w, h}; shape corners are inclusive.
- Layers are 0-based with 0 at the BOTTOM of the stack; a group appears right below its children. Layers can
  also be addressed by exact name. Frames are 0-based. (Lua scripts are 1-based for layers/frames/tags.)
- Tools without `layer`/`frame` use the document's active layer/frame; new layers/frames become active.
  doc_set_active changes them. Check doc_info when unsure.
- Colors: "#RRGGBB" or "#RRGGBBAA" (also "#RGB", "rgb(...)", CSS basic names, "transparent"). Results print
  "#rrggbbaa". The default "simple" ink replaces pixels; a transparent color erases.
- Headless documents pile up: close them with doc_close (all_headless=true closes them all); past 32 open ones the
  oldest unmodified are closed automatically and the result `note` says so.
- Every mutating call is exactly one undo step (undo / redo / history). Wrap a multi-call edit in
  transaction begin/end to make it one step. Failed calls change nothing.
- While a selection exists, drawing, fills, gradients and filters only touch selected pixels. select none clears it.

## Live mode (the Pixmith app)
- When the Pixmith app is open on this computer, the tools edit ITS documents: the user watches every edit appear in
  the window, and each call is one undo step named "AI: <tool>" that they can take back with Cmd/Ctrl+Z. Without
  the app everything works headless, in documents kept inside this server (ids "d1", "d2", ...).
- app_status tells which one you have: connected or not, the app's documents (ids "app:<id>", e.g. "app:doc-3") and
  the active one. app_connect / app_disconnect switch at runtime (use app_connect after the user starts the app).
- In live mode `doc` may be omitted to mean the app's ACTIVE document, so the user chooses the target by switching
  tabs. doc_new / doc_open create the sprite in the app (target "headless" keeps it inside the server instead);
  doc_save / doc_close / doc_set_active act on the app's tab. doc_list shows both kinds.
- Once the app has been connected in this session, a call that creates or edits a document never silently turns
  headless: if the app is gone it reconnects once and otherwise returns an error. Start the app again, pass
  target "headless" (doc_new / doc_open) or a headless `doc` id, or call app_disconnect to work headless on purpose.
- Tools default to the app's current layer and frame; aiming a tool at another layer or frame moves the app's
  current one there, so the user sees where you are working. The user may edit at the same time: a call that lost
  that race is repeated on the new version automatically, and a busy app (open dialog) is retried for a few seconds.
- Not available on app documents: history, history_goto (the app owns the undo history; transaction does nothing,
  every call is its own step). undo / redo work when the app supports them and otherwise tell the user to press
  Cmd/Ctrl+Z. The app's own selection is not visible to tools; a selection made with the select tool lives in this
  server until someone else changes the document.
- Prefer a few meaningful calls over thousands of tiny ones: every call is a full round trip to the window and an
  undo step. draw_pixels (batches), draw_polygon and run_lua are good for bigger drawings.

## Looking at your work
- render returns a PNG you can see (auto-scaled, checkerboard = transparent). Render after each meaningful step.
- get_pixels returns exact colors as a character grid + legend (great for checking outlines and palettes).
- doc_info lists layers, frames, cels (which have pixels), tags, palette, slices, tilesets, selection, history.

## Typical workflows
- New sprite: doc_new (width, height, optionally palette_preset) -> palette (load_preset / replace_all) ->
  layer add (e.g. "outline", "color") -> draw_* / fill -> render -> doc_save path=... .pxmt or .aseprite.
- Animation: draw frame 0 -> frame duplicate -> edit the new frame -> repeat -> frame set_duration ->
  tag add (name, from, to, direction) -> render frames=[...] or onion_skin to compare -> export gif / sheet.
- Tween instead of redrawing: tween set_keys at two frames (x/y/rotation/scale/opacity, ease); render and
  exports show the motion; tween bake turns it into editable cels.
- Tilemap: tilemap add_layer (tile size) -> draw tiles via tileset set_tile or by drawing on the layer ->
  tilemap stamp. Auto-tiling: terrain create (mode) -> terrain generate_template -> paint the template
  tiles -> terrain assign_masks -> terrain_paint paint/rect/flood -> terrain missing to check coverage.
- Sprites traced from AI reference art: doc_open format=trace_grid (or trace on an open cel) puts pseudo-pixel art on a real
  grid; filter remove_background / clean_edges tidy it; lint lists what is still wrong; frame_diff / render diff_prev find
  walk-cycle parts that did not move; warp bends a cel without blur. Help topic cleanup.
- Isometric maps: tilemap add_layer with cell_w, cell_h, offset (e.g. 96 x 48 at [592, 360]) -> tilemap stamp {i, j, tile} ->
  tileset set_pivot for tall tiles -> terrain / terrain_paint -> export_godot_tileset (ISOMETRIC / DIAMOND_DOWN). Help topic tilemaps.
- Godot: export_godot (project_root with project.godot, tres_path inside it; one animation per tag) and
  export_godot_tileset for tilesets with terrains. preview=true shows what would be written.
- Godot Live Link (Pixmith app 0.5.1 or later): export_godot once with save_settings=true; then a running game with the
  Pixmith Live Link addon (debug builds) updates its sprites after every edit you make, no re-export. game_screenshot
  returns a screenshot of that running game (scale 1-8 enlarges small pixel games) so you can check the result.
- Replace one layer's pixels from a PNG in one undo step: layer action import_png with replace=true (path, layer, frame,
  optional x / y). More how-tos per tool group: help topics app_tools (the app's tool names), documents, layers, drawing, selection,
  transform, filters, cleanup, tilemaps, slices, tween, export, history, live.
- Anything else: run_lua (see help topic lua_api); timeout_seconds (1-120) allows longer scripts.
"##;

const DOCUMENTS: &str = r##"# Documents (doc_new, doc_open, doc_save, doc_close, doc_list, doc_info, doc_set_active)
- doc_new {width, height, name?, background?, color_mode?, palette_preset?}; doc_open {path} reads .pxmt / .aseprite and
  imports .png, .gif, sprite sheets (cell_width / cell_height) and Aseprite JSON sheets. Both return the document id.
  doc_open {path, format: "trace_grid", pitch?, colors?, remove_background?, shadow?, close_outline?, crop?} (File > Import > Trace Pixel Art…)
  traces pseudo-pixel art (AI image tools) onto its real grid into a new document; the result's `trace` has pitch, phase, palette, quality.
- doc_save {path?, copy?, replace?}: .pxmt keeps everything, .aseprite / .ase for Aseprite; copy=true leaves the document as is.
  Without path an imported document (PNG, GIF, sheet) is saved next to the imported file as <name>.pxmt (the result says where).
  The app refuses to save to a file another tab has open: replace=true closes that tab first when it has no unsaved changes.
  Use export for PNG / GIF / sheets (export may also write into a folder given with the server's --allow-dir).
- doc_close {doc} refuses unsaved changes unless discard_changes=true. doc_close {all_headless: true} closes every
  headless document (d1, d2, ...) at once and keeps the unsaved ones unless discard_changes=true.
- Headless documents are limited to 32: when doc_new / doc_open would pass that, the oldest unmodified (saved or never
  edited) ones are closed and the result `note` lists them. Close what you are done with.
- doc_info {sections?} shows layers, frames, cels, tags, palette, slices, tilesets, selection, history; doc_set_active
  {layer?, frame?} sets the defaults tools use without `layer` / `frame`.
"##;

const LAYERS: &str = r##"# Layers, frames, cels and tags (layer, frame, cel, tag)
- layer {action}: add, add_group, duplicate, remove (alias delete), move_up / move_down / move_to, into_group /
  out_of_group / wrap_in_group, merge_down, set {name, visible, opacity, blend_mode}, import_png {path}, import_sheet
  {path, cell_width, cell_height, layer | new_layer, frame}: cuts a sprite sheet into consecutive frames (created as needed) of
  a layer in one undo step.
- Replace the pixels of an existing layer from a PNG in ONE undo step: layer {action: "import_png", layer, frame, path,
  replace: true, x?, y?}. Name, order, opacity and blend mode stay; without replace a new layer is added.
- frame {action}: add, duplicate, remove (alias delete), move, set_duration {duration_ms, frame | frames | all}.
- cel {action}: link, unlink, set_opacity, clear (aliases remove, delete), copy, paste, move; cels are one layer in one frame.
  copy / paste / move return {copied|pasted|moved: {layers: [a, b], frames: [a, b], cels: n}} and a summary line.
- tag {action}: add {name, from, to, direction?, repeat?, color?}, update, remove (alias delete).
- Layer 0 is the bottom, frames are 0-based, layers can be given by name. New layers and frames become active.
"##;

const DRAWING: &str = r##"# Drawing (draw_pixels, draw_shape, draw_polygon, draw_curve, draw_stroke, fill, spray, retouch, gradient, draw_text, brush)
- They take optional doc, layer and frame, a color ("#RRGGBB[AA]", names, "transparent" erases) and, where it makes
  sense, brush_size / brush_shape, ink (simple, alpha, lock_alpha, behind, shading) and mirror (horizontal / vertical /
  both); see each tool's schema.
- Use draw_pixels for exact batches ({pixels: [{x, y, color}]} or points + one color), draw_shape for lines, rectangles
  and ellipses (filled or outline), draw_polygon / draw_curve / draw_stroke for paths (pixel_perfect for clean lines).
- fill is the Paint Bucket (contiguous or global, tolerance, dither pattern); gradient and spray respect the selection.
- draw_stroke is the Pencil and, with color "transparent", the Eraser. retouch is the Blur and Jumble brushes (kind, points,
  size, strength; one undo step). The full app-tool to MCP-call table is help topic app_tools.
- With a selection active only selected pixels change (select none clears it). Check results with render / get_pixels.
- For anything bigger than a few hundred calls, write one run_lua script instead.
"##;

const SELECTION: &str = r##"# Selection and clipboard (select, iso_grid, clipboard)
- select {action}: rect (Marquee), polygon (Polygonal Lasso), lasso, wand (Magic Wand), by_color, iso_tile (Isometric Tile:
  x, y or col, row; tile_w / tile_h default 32 x 16; offset_x / offset_y), all, none, invert, expand, contract, border; mode replace / add /
  subtract. delete clears the selected pixels, move_pixels shifts them, crop crops the canvas to the selection.
- iso_grid {action: get | set | reset, tile_w, tile_h, offset_x, offset_y, layer?}: the document's isometric grid, the default of select iso_tile.
  With an isometric tilemap layer (tilemap add_layer cell_w) its grid IS the layer's (source "layer"): get reads it, set moves its offset,
  and select iso_tile uses it by default; otherwise the server keeps a per-document setting (default 32 x 16 at 0, 0).
- clipboard {action}: copy, cut, paste, paste_image (from a PNG file), move {dx, dy}, commit, cancel. A paste is a
  floating selection until committed; the next edit commits it too.
- Drawing, fills, gradients and filters only touch selected pixels while a selection exists.
"##;

const TRANSFORM: &str = r##"# Transform (transform, free_transform, sprite_size)
- transform {action}: flip_horizontal, flip_vertical, rotate_cw / rotate_ccw / rotate_180, rotate with angle (RotSprite);
  acts on the selection, otherwise the cel; all_frames / all_layers widen it.
- free_transform: begin, set x / y / width / height / angle / skew_x / skew_y (anchor for resizing), info, commit or cancel.
- sprite_size {action: scale | canvas}: resample the whole sprite, or change the canvas size (anchor or offsets).
"##;

const FILTERS: &str = r##"# Filters (filter)
- filter {kind, target: cel | layer | frame, region?}: replace_color, invert, desaturate, hsl, brightness_contrast,
  color_balance, curves, outline, drop_shadow, remove_isolated_pixels, blur, sharpen, clean_edges (FX > Clean Edges…),
  remove_background (FX > Remove Background…). Limited to the selection or `region`. See help topic cleanup.
- preview=true returns an image of the result without changing the document.
"##;

const CLEANUP: &str = r##"# Cleanup of sprites made from reference art (lint, trace, filter clean_edges / remove_background, frame_diff, warp)
- Trace pseudo-pixel art (AI image tools: soft upscaled edges, drifting grid, paper background, drop shadow) onto a real grid:
  doc_open {path, format: "trace_grid", pitch: "auto" | 6.5 | [x, y], colors: 16 | "auto" | "keep" | [...], remove_background, shadow,
  close_outline, crop} (File > Import > Trace Pixel Art…) or trace {…} on an open document's cel (every cel is resampled with the active
  cel's grid so an animation stays on one grid; preview=true / detect_only=true change nothing). Results: pitch, phase, palette, quality (0-1).
- filter {kind: "remove_background", tolerance?, shadow?, global?, color?} (FX > Remove Background…): flood fill from the border, equal colors enclosed by the figure stay.
- filter {kind: "clean_edges", halo?, despeckle?, holes?, light_specks?, thin_outline?, thickness?} (FX > Clean Edges…): never removes interior detail
  (eyes, 1 px highlights, fingers); name no pass for halo + despeckle + thin_outline, otherwise only the named ones run; target layer / frame widen it.
- lint {target?, speck?, list?, palette?} (FX > Lint Sprite…): read-only per-cel report: light_edge, holes, specks, no_outline, strays,
  semi_transparent, off_palette, each {count, items: [{x, y, size}]} (positions capped by `list`).
- frame_diff {frame | frames | all, layer?, wrap?, image?} (View > Frame Difference): changed / appeared / disappeared / total / bounds /
  reference_frame versus the previous frame; render {diff_prev: true, diff_opacity?, diff_wrap?, diff_layer?} tints those pixels over the render.
- warp {grid: [cols, rows], offsets: [[dx, dy], ...], samples?, smooth?, pins?, preview?} (Edit > Warp…): a puppet / mesh warp of the cel (selection-aware),
  resampled by majority vote so nothing blurs and no color is invented.
"##;

const TILEMAPS: &str = r##"# Tilemaps and terrain (tileset, tilemap, terrain, terrain_paint)
- tilemap {action: add_layer, convert, to_image, assign_tileset, stamp, get_tile, grid}: a tilemap layer shows a grid of
  tiles from a tileset; tile 0 is always empty.
- tileset {action}: list, image, get_tile, set_tile {rows + legend | path}, add_tile, duplicate_tile, delete_tile (alias
  remove_tile), rename, set_base_index.
- Auto-tiling: terrain create {mode: sides | blob | corners} -> generate_template -> paint the template tiles ->
  assign_masks -> terrain_paint {paint | rect | flood | erase | override | pick} -> terrain missing must report none.
- terrain {action}: create, set_mode, add, remove (alias delete), edit, move, list, set_tile, get_tile, set_options,
  rebuild, clear_overrides, missing.
- Tilemap iso (isometric, 2:1 Godot DIAMOND_DOWN): tilemap {action: "add_layer", cell_w: 96, cell_h: 48, offset: [592, 360]} creates an
  isometric layer + tileset (tile images default to the cell size; give tile_width / tile_height for taller tiles). Cell (i, j) has its box at
  (offset_x + (i - j) * cell_w / 2, offset_y + (i + j) * cell_h / 2); i runs down-right, j down-left; cells draw back to front.
  tilemap stamp {i, j, tile, flip_x?, flip_y?} and get_tile {i, j} (or a canvas pixel x, y; get_tile then also gives `seen`, the front-most
  visible tile), grid (rows = j, columns = i), offset {offset?: [x, y]} reads or moves the layer's grid (layers at other elevations).
  tileset {action: set_iso {cell_w, cell_h} | get_pivot | set_pivot {tile, pivot: [x, y]} | clear_pivot | pivots | texture_origin}: the pivot
  is the point of a tall tile's image that sits on the diamond's center (default: centered, bottom-aligned); texture_origin is Godot's value.
  terrain / terrain_paint work on iso layers (sides, blob). iso_grid and select iso_tile share the layer's grid. export_godot_tileset writes
  the isometric TileSet (and, with write_scene, the map with each layer's offset).
"##;

const SLICES: &str = r##"# Slices (slice, user_data)
- A slice is a named rectangle on the canvas: use it for UI 9-patches, hit boxes, icons cut from a sheet and pivots.
  Slices are exported with sprite sheets (export format sheet, json=true) and Godot exports.
- slice {action}: list, add {name, bounds: {x, y, w, h}}, set {slice, name?, bounds?, center?, pivot?, clear_center?,
  clear_pivot?, frame?}, remove {slice} (alias delete). `slice` is an index or a name.
- center is the 9-patch middle rectangle and pivot is [x, y], both relative to the slice's top-left.
- Bounds, center and pivot are keyed per frame: a value set at `frame` applies from there until the next key (default 0).
- user_data {target: slice, slice, text?, color?} attaches notes or a color to a slice (also to the sprite, layers, cels, tags).
- doc_info {sections: ["slices"]} lists them; render and get_pixels do not draw slices.
"##;

const TWEEN: &str = r##"# Tweening (tween)
- Keys on a layer (or group) for x, y, rotation, scale_x, scale_y, opacity; the layer renders and exports animated.
- tween {action}: set_key / set_keys {frame, values}, remove_key (alias delete_key), move_key, set_ease, set_pivot,
  reset_pivot, set_options, copy_keys / paste_keys, info, bake / bake_all (turn the motion into editable cels).
- Rig (Tween Parent): tween {action: set_parent {layer, parent} | clear_parent | get_parent | children | composed_transform {frame}}: a child's
  transform is its parent's composed with its own about its own pivot (thigh -> shin -> boot, body -> arm). bake / bake_all are rig-aware.
- Eases: linear, hold, in_quad, out_quad, in_out_quad, in_out_cubic, out_back, out_bounce, bezier [x1, y1, x2, y2].
"##;

const EXPORT: &str = r##"# Export (export, export_godot, export_godot_tileset)
- export {format: png | gif | apng | png_sequence | sheet, path, scale?, tag?, split_tags?, layers?, trim?}; sheets take
  layout / columns / padding / border / extrude and json=true for Aseprite-style metadata. dry_run=true only reports.
- export_godot {project_root, tres_path}: Godot 4 SpriteFrames (+ atlas, + .tscn with tscn=true), one animation per
  tag; preview=true shows what would be written; save_settings=true remembers the settings (Live Link needs it).
- export_godot_tileset: TileSet .tres with terrains and collisions. Save the document with doc_save too.
"##;

const HISTORY: &str = r##"# History (undo, redo, history, history_goto, transaction)
- Every mutating call is one labeled undo step; failed calls change nothing.
- undo / redo {steps}; history lists the steps; history_goto {position} jumps (0 = as opened).
- transaction {action: begin, label} ... transaction {action: end} makes many calls one undo step (nestable).
- With the Pixmith app connected the app owns the history: edits are steps named "AI: <tool>"; history, history_goto and
  transaction are not available there.
"##;

const LIVE: &str = r##"# Live mode (app_status, app_connect, app_disconnect, game_screenshot)
- With the Pixmith app running, the tools edit ITS documents (ids "app:<id>"); `doc` omitted = the app's active one.
  app_status shows the connection and the headless documents; app_connect / app_disconnect switch at runtime.
- doc_new / doc_open create the sprite in the app; target "headless" keeps it inside this server (ids d1, d2, ...).
- Once connected in a session, a call that creates or edits a document never silently turns headless: if the app is
  gone you get an error (the server reconnects once first). Start the app again, pass target "headless" / a headless
  `doc` id, or call app_disconnect to work headless on purpose.
- game_screenshot returns the running Godot game (Live Link addon, Pixmith 0.5.1+).
"##;

const APP_TOOLS: &str = r##"# Pixmith app tools -> MCP calls
Each line: what the app calls it -> the MCP call (example). Coordinates are canvas pixels; colors "#RRGGBB[AA]".

## Select and move
- Rectangular Marquee -> select {action: "rect", x: 2, y: 2, w: 8, h: 6}
- Lasso -> select {action: "lasso", points: [[2,2],[9,3],[5,9]]}
- Polygonal Lasso -> select {action: "polygon", points: [[2,2],[9,3],[5,9]]}
- Magic Wand -> select {action: "wand", x: 4, y: 4, tolerance: 0, contiguous: true}; Select by Color -> select {action: "by_color", x, y}
- Select > Isometric Tile -> select {action: "iso_tile", x: 32, y: 16} or {col: 1, row: 0}; the grid is the active isometric tilemap layer's (cell size and offset) when there is one, else the document's iso_grid {action: "set", tile_w: 96, tile_h: 48, offset_x: 592, offset_y: 360} (default 32 x 16 at 0, 0; kept by this server per document, not read from the app), or pass tile_w / tile_h / offset_x / offset_y to one call
- Add / subtract with Shift / Alt -> mode: "add" | "subtract" on any select action; Select All / Deselect / Invert -> select all | none | invert
- Select > Modify (Expand, Contract, Border) -> select {action: "expand", amount: 2}
- Move (move the selected pixels) -> select {action: "move_pixels", dx: 3, dy: 0}; move a floating paste -> clipboard {action: "move", dx, dy}
- Cut / Copy / Paste -> clipboard {action: "copy" | "cut" | "paste" | "commit"}; Delete -> select {action: "delete"}; Crop to Selection -> select {action: "crop"}
- Free Transform -> free_transform {action: "begin"}, then {action: "set", angle: 15}, then {action: "commit"}; Flip / Rotate 90 -> transform {action: "flip_horizontal"}

## Paint
- Pencil -> draw_stroke {points: [[1,1],[6,4]], color: "#ff0000"} (single pixels: draw_pixels {points, color})
- Eraser -> the same call with color "transparent"; erase a region: select it, then select {action: "delete"}
- Spray -> spray {points: [[8,8]], color, radius: 4, density: 20, seed: 1}
- Blur and Jumble -> retouch {kind: "blur" | "jumble", points: [[2,2],[12,2]], size: 5, strength: 50, seed?}
- Paint Bucket -> fill {x, y, color, contiguous: true, tolerance: 0, pattern?}
- Gradient -> gradient {from: [0,0], to: [0,15], color_from, color_to, kind: "linear" | "radial", dither?}

## Shapes and text
- Line / Rectangle / Ellipse -> draw_shape {shape: "line" | "rectangle" | "ellipse", from, to, color, filled?}
- Curve -> draw_curve {points: [start, control1, control2, end], color}
- Polygon -> draw_polygon {points: [[1,1],[8,2],[4,9]], color, filled?, closed?}
- Contour (freehand outline, filled) -> draw_polygon {points: <the traced path>, color, filled: true}
- Text -> draw_text {text: "HI", x, y, color, scale?}
- Eyedropper -> get_pixel {x, y} (composite plus every layer's value); a whole area: get_pixels
- Slice -> slice {action: "add", name, bounds: {x, y, w, h}}

## Tool options
- Ink modes Simple / Alpha Compositing / Lock Alpha / Behind / Shading -> ink: "simple" | "alpha" | "lock_alpha" | "behind" | "shading" (+ shading_ramp) on any draw_* call, fill, spray and gradient
- Pixel-perfect Pencil -> pixel_perfect: true on draw_stroke
- Symmetry (Mirror Horizontally / Vertically) -> mirror: "horizontal" | "vertical" | "both"
- Brush size / Round Brush -> brush_size: 1-64, brush_shape: "circle" (retouch takes `size`)
- Custom brush (Brush from Selection, Reset Brush) -> select pixels, then brush {action: "capture"}; brush {action: "reset"}
- Filled Shapes -> filled: true; Contiguous Fill -> contiguous on fill

## Sprite, layers, animation
- Layers / groups / blend modes -> layer {action: "add" | "set" | "merge_down" | ...}; Frames -> frame; Cels and links -> cel (copy / paste / move report the block as {copied|pasted|moved: {layers: [a, b], frames: [a, b], cels: n}}); Tags -> tag
- Sprite sheet PNG into consecutive frames of a layer -> layer {action: "import_sheet", path, cell_width: 32, cell_height: 32, layer, frame: 0, new_layer?}
- Palette, color mode, Snap Pixels to Palette -> palette, color_mode; Sprite > Canvas Size -> sprite_size; FX menu -> filter
- Tween -> tween; Tween panel Parent (rig) -> tween {action: "set_parent", layer, parent} (also get_parent, children, composed_transform); User Data -> user_data
- Onion Skin -> render {onion_skin: {before: 1, after: 1}} (a view only: it never changes the document)
- View > Frame Difference -> render {diff_prev: true, diff_opacity: 0.65} (tints the pixels changed from the previous frame: magenta changed, green appeared, red disappeared) and frame_diff {frame, layer?, wrap?} for the counts and bounds
- History panel -> history, history_goto, undo, redo; File > Scripts -> run_lua (help topic lua_api)

## Cleanup and warp
- FX > Clean Edges… -> filter {kind: "clean_edges", halo?, despeckle?, holes?, light_specks?, thin_outline?}
- FX > Lint Sprite… -> lint {target?, speck?, list?, palette?} (read-only)
- FX > Remove Background… -> filter {kind: "remove_background", tolerance?, shadow?, global?, color?}
- File > Import > Trace Pixel Art… -> doc_open {path, format: "trace_grid", pitch?, colors?, remove_background?, shadow?, close_outline?, crop?}; on an open document -> trace {…}
- Edit > Warp… -> warp {grid: [3, 3], offsets: [[dx, dy], ...], pins?, smooth?, samples?}

## Tilemaps
- Tilemap layers and Tile Stamp mode -> tilemap {action: "add_layer" | "stamp" | ...}, tileset
- Tilemap iso (isometric layers) -> tilemap {action: "add_layer", cell_w: 96, cell_h: 48, offset: [592, 360]}, tilemap {action: "stamp", i, j, tile}, tilemap {action: "offset", offset: [x, y]}, tileset {action: "set_pivot", tile, pivot: [x, y]} (also set_iso, get_pivot, clear_pivot, pivots, texture_origin)
- Terrain brush -> terrain (setup), terrain_paint {action: "paint", points} (also on isometric layers)

## Files, Godot and Live Link
- File > Open / New / Save / Import -> doc_open (.pxmt, .aseprite, PNG, GIF, sheets), doc_new, doc_save (an imported, never saved document is saved next to the imported file as .pxmt; replace: true first closes another clean tab that has the target file open)
- Export PNG / GIF / APNG / PNG sequence / Sprite Sheet -> export {format}
- Export to Godot / Export Tileset to Godot -> export_godot, export_godot_tileset (preview: true to dry-run; isometric tilesets export as ISOMETRIC / DIAMOND_DOWN)
- Live Link (running Godot game) -> export_godot {save_settings: true} once, then game_screenshot; the Pixmith app itself -> app_status, app_connect

## View-only in the app (no effect on the document, nothing to call)
- Grid, Isometric Grid display, Tiled Mode, Preview, zoom, View Options, Preferences. render shows only onion_skin and frame difference (diff_prev) of these. For the isometric grid, select iso_tile gives the cells' pixel masks and tilemap grid / get_tile the cells' tiles.

## Not available through this server
- Reference layers (New Reference Layer, Show Reference Image), Assign sRGB / Remove Profile, export or import of WebP, JPEG, BMP, TGA, ICO, PCX and FLC (PNG, GIF, APNG and .aseprite only), plugin commands and keyboard shortcut editing.
"##;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Help",
        "help",
        "Pixmith help",
        READ,
        HelpArgs,
        help,
        "Documentation for using this server: guide (conventions and workflows — read this first), lua_api (the full Lua scripting reference for run_lua), lua_examples (example scripts shipped with Pixmith), palettes (palette and color mode tools, built-in preset names), tools (one line per tool), app_tools (every tool of the Pixmith app - Pencil, Eraser, Paint Bucket, Blur, Marquee, Isometric Tile, ink modes, ... - mapped to its MCP call), and short how-tos per tool group: documents, layers, drawing, selection, transform, filters, cleanup (trace, lint, clean edges, remove background, frame difference, warp), tilemaps (incl. isometric), slices, tween (incl. the parent rig), export, history, live."
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Guide,
    LuaApi,
    LuaExamples,
    Palettes,
    Tools,
    AppTools,
    Documents,
    Layers,
    Drawing,
    Selection,
    Transform,
    Filters,
    Cleanup,
    Tilemaps,
    Slices,
    Tween,
    Export,
    History,
    Live,
}

#[derive(Deserialize, JsonSchema)]
pub struct HelpArgs {
    /// guide (default), lua_api, lua_examples, palettes, tools, app_tools (the Pixmith app's tools/menu items mapped to MCP calls), or a tool group: documents, layers, drawing, selection,
    /// transform, filters, cleanup, tilemaps, slices, tween, export, history, live.
    pub topic: Option<Topic>,
}

pub fn examples_text() -> String {
    let mut out =
        String::from("# Example Lua scripts shipped with Pixmith\n\nRun one with run_lua (source = the code).\n");
    for (name, src) in super::script::examples() {
        out.push_str(&format!("\n## {name}\n\n```lua\n{}\n```\n", src.trim_end()));
    }
    out
}

pub fn tools_text() -> String {
    let mut out = String::from("# Tools\n");
    let mut group = "";
    for t in super::all() {
        if t.group != group {
            group = t.group;
            out.push_str(&format!("\n## {group}\n"));
        }
        let first = t.description.split(". ").next().unwrap_or(t.description);
        out.push_str(&format!("- {}: {}\n", t.name, first.trim_end_matches('.')));
    }
    out
}

/// The text of a topic.
fn topic_text(t: Topic) -> String {
    match t {
        Topic::Guide => GUIDE.to_string(),
        Topic::LuaApi => LUA_API.to_string(),
        Topic::LuaExamples => examples_text(),
        Topic::Palettes => format!(
            "Palette and color mode: palette {{action}} (get, set, add, insert, remove, move, duplicate, replace_all, sort, ramp, from_sprite, list_presets, load_preset, load_file, save_file) and color_mode {{mode: rgba | grayscale | indexed, dither?, conform_only?}}.\n\nBuilt-in palette presets (palette action load_preset, or doc_new palette_preset):\n- {}",
            super::docs::preset_names().join("\n- ")
        ),
        Topic::Tools => tools_text(),
        Topic::AppTools => APP_TOOLS.to_string(),
        Topic::Documents => DOCUMENTS.to_string(),
        Topic::Layers => LAYERS.to_string(),
        Topic::Drawing => DRAWING.to_string(),
        Topic::Selection => SELECTION.to_string(),
        Topic::Transform => TRANSFORM.to_string(),
        Topic::Filters => FILTERS.to_string(),
        Topic::Cleanup => CLEANUP.to_string(),
        Topic::Tilemaps => TILEMAPS.to_string(),
        Topic::Slices => SLICES.to_string(),
        Topic::Tween => TWEEN.to_string(),
        Topic::Export => EXPORT.to_string(),
        Topic::History => HISTORY.to_string(),
        Topic::Live => LIVE.to_string(),
    }
}

fn help(_: &mut Engine, a: HelpArgs) -> R<Out> {
    Out::text(topic_text(a.topic.unwrap_or(Topic::Guide)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Topic; 19] = [
        Topic::Guide,
        Topic::LuaApi,
        Topic::LuaExamples,
        Topic::Palettes,
        Topic::Tools,
        Topic::AppTools,
        Topic::Documents,
        Topic::Layers,
        Topic::Drawing,
        Topic::Selection,
        Topic::Transform,
        Topic::Filters,
        Topic::Cleanup,
        Topic::Tilemaps,
        Topic::Slices,
        Topic::Tween,
        Topic::Export,
        Topic::History,
        Topic::Live,
    ];

    #[test]
    fn every_tool_is_covered_by_some_help_topic() {
        let help: String = ALL.iter().filter(|t| **t != Topic::Tools).map(|t| topic_text(*t)).collect();
        for t in super::super::all() {
            assert!(help.contains(t.name), "no help topic mentions the tool {}", t.name);
        }
    }

    #[test]
    fn every_topic_is_in_the_schema_and_has_text() {
        let schema = crate::schema::input_schema::<HelpArgs>();
        let listed: Vec<String> = serde_json::from_value(schema["properties"]["topic"]["enum"].clone()).unwrap();
        assert_eq!(listed.len(), ALL.len(), "{listed:?}");
        for t in ALL {
            assert!(topic_text(t).len() > 100, "{t:?}");
        }
    }
}
