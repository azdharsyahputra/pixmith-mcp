//! Selection and clipboard. The selection is part of the document (and of its undo history); drawing,
//! filters, gradients and fills are clipped to it.

use super::{ADD, CHANGE, Hints, Out, ToolDef, need, tool};
use crate::doc::fail;
use crate::engine::{Clip, DocEntry, Engine, R, Ref, Undo};
use crate::iso::Iso;
use pixmith_core::document::tilemap as tm;
use pixmith_core::paint::selection as sel;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Selection",
            "select",
            "Select / modify selection",
            CHANGE,
            SelectArgs,
            select,
            "Change the selection (one undo step each): rect (Rectangular Marquee: x, y, w, h), lasso (Lasso: freehand outline points), polygon (Polygonal Lasso: corner points), wand (Magic Wand at x, y with tolerance; contiguous by default), by_color (Select by Color: every pixel matching the color at x, y), iso_tile (Select > Isometric Tile: the diamond cell of an isometric grid, by a pixel x, y or by col, row; the grid is the active isometric tilemap layer's (or `layer`'s) when there is one, else the document's iso_grid settings, default 32x16 at 0, 0; or give tile_w, tile_h, offset_x, offset_y), all, none, invert, expand/contract/border (by `amount` pixels). mode = replace (default), add or subtract combines with the existing selection. Also edits the selected pixels: delete (clear them), move_pixels (the Move tool: shift the selected pixels by dx, dy), crop (crop the canvas to the selection bounds). Returns the selection bounds. Drawing and filters only affect selected pixels while a selection exists; select none to remove it."
        ),
        tool!(
            "Selection",
            "iso_grid",
            "Isometric grid settings",
            SETTING,
            IsoGridArgs,
            iso_grid,
            "Read or set the isometric grid of a document: tile_w x tile_h (even, 4-4096; default 32 x 16) and offset_x / offset_y (where cell (0, 0)'s bounding box starts; default 0, 0). Cell (col, row) has its box at (offset_x + (col - row) * tile_w / 2, offset_y + (col + row) * tile_h / 2). `select` action iso_tile uses these as its defaults, so set the grid once. When the document has an isometric tilemap layer (tilemap add_layer with cell_w; Tilemap iso) the grid IS that layer's: iso_grid get reports its cell size and offset (`source` \"layer\"; the layer is `layer`, else the active layer if it is isometric, else the only isometric layer), set moves the layer's grid offset (offset_x / offset_y; the cell size comes from its tileset, see tileset action set_iso) and select iso_tile uses it by default, so the overlay, the tilemap and the selection share one offset. Without an isometric layer the settings are kept by this server for the document (not in the file or in the app: the app's Isometric Grid view settings cannot be read or written through the live protocol) and are lost when the server stops or the document is closed. Actions: get (default), set (only the fields you give change), reset (forgets the server's setting)."
        ),
        tool!(
            "Selection",
            "clipboard",
            "Copy / cut / paste",
            ADD,
            ClipArgs,
            clipboard,
            "Edit > Cut / Copy / Paste (clipboard shared by all documents): copy or cut the selected pixels of a layer (the whole canvas when nothing is selected); paste them (or a PNG file with paste_image) as a floating selection at (x, y) (default: where they were copied from); move the floating pixels by (dx, dy) like the Move tool; commit to drop them onto the layer, or cancel to discard the paste. A floating paste is also committed by the next edit."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SelectAction {
    Rect,
    Polygon,
    Lasso,
    Wand,
    ByColor,
    IsoTile,
    All,
    None,
    Invert,
    Expand,
    Contract,
    Border,
    Delete,
    MovePixels,
    Crop,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SelMode {
    #[default]
    Replace,
    Add,
    Subtract,
}

#[derive(Deserialize, JsonSchema)]
pub struct SelectArgs {
    pub action: SelectAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// replace (default), add or subtract — for rect, polygon, lasso, wand, by_color, iso_tile (the app's Shift = add, Alt = subtract).
    pub mode: Option<SelMode>,
    /// rect: left; wand/by_color: sample point x.
    pub x: Option<i32>,
    /// rect: top; wand/by_color: sample point y.
    pub y: Option<i32>,
    /// rect: width.
    pub w: Option<u32>,
    /// rect: height.
    pub h: Option<u32>,
    /// polygon/lasso: outline [[x, y], ...] (at least 3).
    pub points: Option<Vec<[i32; 2]>>,
    /// wand/by_color: 0-255 per-channel tolerance (default 0).
    pub tolerance: Option<u32>,
    /// wand: connected pixels only (default true).
    pub contiguous: Option<bool>,
    /// expand/contract/border: pixels (default 1).
    pub amount: Option<u32>,
    /// move_pixels: horizontal offset.
    pub dx: Option<i32>,
    /// move_pixels: vertical offset.
    pub dy: Option<i32>,
    /// iso_tile: cell column (with row); alternative to x, y.
    pub col: Option<i64>,
    /// iso_tile: cell row (with col).
    pub row: Option<i64>,
    /// iso_tile: tile width in pixels, even, 4-4096 (default: the isometric layer's cell, else the iso_grid setting, else 32).
    pub tile_w: Option<i32>,
    /// iso_tile: tile height in pixels, even, 4-4096 (default: the isometric layer's cell, else the iso_grid setting, else 16).
    pub tile_h: Option<i32>,
    /// iso_tile: x of the grid's cell (0, 0) (default: the isometric layer's offset, else the iso_grid setting, else 0).
    pub offset_x: Option<i32>,
    /// iso_tile: y of the grid's cell (0, 0) (default: the isometric layer's offset, else the iso_grid setting, else 0).
    pub offset_y: Option<i32>,
    /// Layer sampled by wand/by_color and edited by delete/move_pixels (default: active); iso_tile: the isometric tilemap layer whose grid to use.
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
}

fn grid_of(grid: Option<&Iso>) -> Iso {
    grid.copied().unwrap_or_else(|| Iso::new(crate::iso::DEFAULT_W, crate::iso::DEFAULT_H, 0, 0))
}

/// Changes a setting the server keeps, not the document.
const SETTING: Hints = Hints { read_only: false, destructive: false, idempotent: true, files: false };

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Default, Debug)]
#[serde(rename_all = "snake_case")]
pub enum IsoGridAction {
    #[default]
    Get,
    Set,
    Reset,
}

#[derive(Deserialize, JsonSchema)]
pub struct IsoGridArgs {
    /// get (default), set or reset.
    pub action: Option<IsoGridAction>,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// set: tile width in pixels (even, 4-4096).
    pub tile_w: Option<i32>,
    /// set: tile height in pixels (even, 4-4096).
    pub tile_h: Option<i32>,
    /// set: x of cell (0, 0)'s bounding box.
    pub offset_x: Option<i32>,
    /// set: y of cell (0, 0)'s bounding box.
    pub offset_y: Option<i32>,
    /// The isometric tilemap layer whose grid to read or move (default: the active layer if isometric, else the only isometric layer).
    pub layer: Option<Ref>,
}

/// The isometric tilemap layer a call is about: `layer` when given (it must be isometric), else the active layer
/// if it is isometric, else the document's only isometric layer.
pub fn iso_layer(d: &DocEntry, layer: &Option<Ref>) -> R<Option<u32>> {
    let raw = d.doc.raw();
    let is_iso = |l: u32| unsafe { tm::pixmith_layer_is_iso(raw, l) };
    if layer.is_some() {
        let l = d.layer(layer)?;
        if !is_iso(l) {
            return Err(format!("layer {l} ({:?}) is not an isometric tilemap layer", d.layer_name(l)));
        }
        return Ok(Some(l));
    }
    let active = d.layer.min(d.layer_count().saturating_sub(1));
    if d.layer_count() > 0 && is_iso(active) {
        return Ok(Some(active));
    }
    let all: Vec<u32> = (0..d.layer_count()).filter(|&l| is_iso(l)).collect();
    Ok(if let [only] = all.as_slice() { Some(*only) } else { None })
}

/// The grid of an isometric tilemap layer.
pub fn layer_grid(d: &DocEntry, l: u32) -> Iso {
    let raw = d.doc.raw();
    let ts = unsafe { tm::pixmith_layer_tileset(raw, l) }.max(0) as u32;
    let (mut cell, mut off) = ([0u32; 2], [0i32; 2]);
    unsafe {
        tm::pixmith_tileset_iso_cell(raw, ts, cell.as_mut_ptr());
        tm::pixmith_layer_grid_offset(raw, l, off.as_mut_ptr());
    }
    Iso::new(cell[0] as i32, cell[1] as i32, off[0], off[1])
}

fn iso_grid(e: &mut Engine, a: IsoGridArgs) -> R<Out> {
    let id = e.doc(&a.doc)?.id.clone();
    let action = a.action.unwrap_or_default();
    let has_values = a.tile_w.is_some() || a.tile_h.is_some() || a.offset_x.is_some() || a.offset_y.is_some();
    if action != IsoGridAction::Set && has_values {
        return Err("tile_w, tile_h, offset_x and offset_y need action set".into());
    }
    if action == IsoGridAction::Set && !has_values {
        return Err("action `set` needs at least one of tile_w, tile_h, offset_x, offset_y".into());
    }
    let layer = iso_layer(e.doc(&a.doc)?, &a.layer)?;
    if let Some(l) = layer {
        let d = e.doc(&a.doc)?;
        let cur = layer_grid(d, l);
        let mut note = None;
        match action {
            IsoGridAction::Get => {}
            IsoGridAction::Reset => {
                note = Some(
                    "reset forgets the server's own setting only; the isometric layer's grid is part of the document (move it with iso_grid set offset_x / offset_y)",
                );
            }
            IsoGridAction::Set => {
                let want = Iso::new(
                    a.tile_w.unwrap_or(cur.w as i32),
                    a.tile_h.unwrap_or(cur.h as i32),
                    a.offset_x.unwrap_or(cur.x as i32),
                    a.offset_y.unwrap_or(cur.y as i32),
                );
                if (want.w, want.h) != (cur.w, cur.h) {
                    return Err(format!(
                        "layer {l}'s cell is {} x {} pixels, set by its tileset (tileset action set_iso changes it while no layer shows tiles of it); iso_grid set moves the grid with offset_x / offset_y",
                        cur.w, cur.h
                    ));
                }
                let raw = d.doc.raw();
                let (x, y) = (want.x as i32, want.y as i32);
                d.edit("Move Isometric Grid", Undo::Step, |_| {
                    unsafe { tm::pixmith_layer_set_grid_offset(raw, l, x, y) }
                        .then_some(())
                        .ok_or_else(|| fail("could not move the grid (offsets are limited to +-16777216 pixels)"))
                })?;
            }
        }
        if action == IsoGridAction::Reset {
            e.iso_grids.remove(&id);
        }
        let d = e.doc(&a.doc)?;
        let g = layer_grid(d, l);
        let mut v = json!({
            "doc": id, "tile_w": g.w, "tile_h": g.h, "offset_x": g.x, "offset_y": g.y,
            "source": "layer", "layer": l, "layer_name": d.layer_name(l),
            "stored": "in the document (the isometric tilemap layer's grid)",
        });
        if let Some(n) = note {
            v["note"] = json!(n);
        }
        return Out::json(v);
    }
    match action {
        IsoGridAction::Get => {}
        IsoGridAction::Reset => {
            e.iso_grids.remove(&id);
        }
        IsoGridAction::Set => {
            let cur = grid_of(e.iso_grids.get(&id));
            let g = Iso::new(
                a.tile_w.unwrap_or(cur.w as i32),
                a.tile_h.unwrap_or(cur.h as i32),
                a.offset_x.unwrap_or(cur.x as i32),
                a.offset_y.unwrap_or(cur.y as i32),
            );
            e.iso_grids.insert(id.clone(), g);
        }
    }
    let g = grid_of(e.iso_grids.get(&id));
    Out::json(json!({
        "doc": id, "tile_w": g.w, "tile_h": g.h, "offset_x": g.x, "offset_y": g.y,
        "source": if e.iso_grids.contains_key(&id) { "setting" } else { "default" },
        "is_default": !e.iso_grids.contains_key(&id),
        "stored": "by this server, per document (not in the file or the app); an isometric tilemap layer (tilemap add_layer cell_w) would take over",
    }))
}

fn select(e: &mut Engine, a: SelectArgs) -> R<Out> {
    let id = e.doc(&a.doc)?.id.clone();
    let grid = e.iso_grids.get(&id).copied();
    let d = e.doc(&a.doc)?;
    let act = super::act(&a.action);
    let mode = a.mode.unwrap_or_default() as u32;
    let raw = d.doc.raw();
    let label = match a.action {
        SelectAction::Rect => "Rectangle Selection",
        SelectAction::Polygon | SelectAction::Lasso => "Lasso Selection",
        SelectAction::Wand => "Magic Wand",
        SelectAction::ByColor => "Select Color",
        SelectAction::IsoTile => "Select Isometric Tile",
        SelectAction::All => "Select All",
        SelectAction::None => "Deselect",
        SelectAction::Invert => "Invert Selection",
        SelectAction::Expand => "Expand Selection",
        SelectAction::Contract => "Contract Selection",
        SelectAction::Border => "Border Selection",
        SelectAction::Delete => "Delete",
        SelectAction::MovePixels => "Move",
        SelectAction::Crop => "Crop to Selection",
    };
    let mut iso_info = None;
    match a.action {
        SelectAction::IsoTile => {
            let from_layer = iso_layer(d, &a.layer)?;
            // Explicit `layer`, else the active isometric layer, else the server's iso_grid setting, else 32 x 16.
            let (saved, source) = match from_layer {
                Some(l) => (layer_grid(d, l), "layer"),
                None if grid.is_some() => (grid_of(grid.as_ref()), "setting"),
                None => (grid_of(None), "default"),
            };
            let g = Iso::new(
                a.tile_w.unwrap_or(saved.w as i32),
                a.tile_h.unwrap_or(saved.h as i32),
                a.offset_x.unwrap_or(saved.x as i32),
                a.offset_y.unwrap_or(saved.y as i32),
            );
            let cell = match (a.col, a.row, a.x, a.y) {
                (Some(c), Some(r), None, None) => (c, r),
                (None, None, Some(x), Some(y)) => g.cell_at(x, y),
                _ => {
                    return Err(
                        "action `iso_tile` needs either `x` and `y` (a pixel inside the cell) or `col` and `row`"
                            .into(),
                    );
                }
            };
            let poly = g.cell_polygon(cell);
            if poly.is_empty() {
                return Err(format!("cell ({}, {}) has no pixels", cell.0, cell.1));
            }
            let flat: Vec<i32> = poly.iter().flat_map(|p| [p.0, p.1]).collect();
            d.edit(label, Undo::Step, |_| {
                unsafe {
                    sel::pixmith_sel_begin(raw, mode);
                    sel::pixmith_sel_poly(raw, flat.as_ptr(), poly.len());
                    sel::pixmith_sel_end(raw);
                }
                Ok(())
            })?;
            iso_info = Some(json!({
                "col": cell.0, "row": cell.1, "tile_w": g.w, "tile_h": g.h, "offset_x": g.x, "offset_y": g.y,
                "grid_source": source, "layer": from_layer,
            }));
        }
        SelectAction::Rect => {
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let (w, h) = (need(a.w, "w", &act)?, need(a.h, "h", &act)?);
            if w == 0 || h == 0 {
                return Err("w and h must be at least 1".into());
            }
            d.edit(label, Undo::Step, |_| {
                unsafe {
                    sel::pixmith_sel_begin(raw, mode);
                    sel::pixmith_sel_rect(raw, x, y, x + w as i32 - 1, y + h as i32 - 1);
                    sel::pixmith_sel_end(raw);
                }
                Ok(())
            })?;
        }
        SelectAction::Polygon | SelectAction::Lasso => {
            let pts = need(a.points.as_ref(), "points", &act)?;
            if pts.len() < 3 {
                return Err("a polygon selection needs at least 3 points".into());
            }
            let flat: Vec<i32> = pts.iter().flat_map(|p| [p[0], p[1]]).collect();
            d.edit(label, Undo::Step, |_| {
                unsafe {
                    sel::pixmith_sel_begin(raw, mode);
                    sel::pixmith_sel_poly(raw, flat.as_ptr(), pts.len());
                    sel::pixmith_sel_end(raw);
                }
                Ok(())
            })?;
        }
        SelectAction::Wand | SelectAction::ByColor => {
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            let contiguous = a.action == SelectAction::Wand && a.contiguous.unwrap_or(true);
            let tol = a.tolerance.unwrap_or(0).min(255);
            d.edit(label, Undo::Step, |_| {
                unsafe { sel::pixmith_sel_wand(raw, layer, frame, x, y, tol, contiguous, mode) }
                    .then_some(())
                    .ok_or_else(|| fail(format!("nothing selected at ({x}, {y}) (outside the canvas?)")))
            })?;
        }
        SelectAction::All => d.edit(label, Undo::Step, |_| {
            let _: () = unsafe { sel::pixmith_sel_all(raw) };
            Ok(())
        })?,
        SelectAction::None => d.edit(label, Undo::Step, |_| {
            let _: () = unsafe { sel::pixmith_sel_clear(raw) };
            Ok(())
        })?,
        SelectAction::Invert => d.edit(label, Undo::Step, |_| {
            let _: () = unsafe { sel::pixmith_sel_invert(raw) };
            Ok(())
        })?,
        SelectAction::Expand | SelectAction::Contract | SelectAction::Border => {
            let kind = match a.action {
                SelectAction::Expand => 0,
                SelectAction::Contract => 1,
                _ => 2,
            };
            let n = a.amount.unwrap_or(1).clamp(1, 1024);
            d.edit(label, Undo::Step, |_| {
                unsafe { sel::pixmith_sel_modify(raw, kind, n) }
                    .then_some(())
                    .ok_or_else(|| fail("there is no selection to modify"))
            })?;
        }
        SelectAction::Delete => {
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            require_selection(d)?;
            d.edit(label, Undo::Step, |_| {
                unsafe { sel::pixmith_sel_delete(raw, layer, frame) }
                    .then_some(())
                    .ok_or_else(|| fail("nothing was deleted"))
            })?;
        }
        SelectAction::MovePixels => {
            let (dx, dy) = (a.dx.unwrap_or(0), a.dy.unwrap_or(0));
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            require_selection(d)?;
            d.edit(label, Undo::Step, |_| {
                unsafe { sel::pixmith_sel_nudge(raw, layer, frame, dx, dy) }
                    .then_some(())
                    .ok_or_else(|| fail("could not move the selection"))
            })?;
        }
        SelectAction::Crop => {
            require_selection(d)?;
            d.edit(label, Undo::Step, |_| {
                unsafe { sel::pixmith_crop_to_selection(raw) }.then_some(()).ok_or_else(|| fail("could not crop"))
            })?;
        }
    }
    let mut v = super::docs::selection_json(d);
    v["doc"] = json!(d.id);
    v["canvas"] = json!([d.doc.width(), d.doc.height()]);
    if let Some(i) = iso_info {
        v["iso_cell"] = i;
    }
    Out::json(v)
}

fn require_selection(d: &DocEntry) -> R<()> {
    if unsafe { sel::pixmith_sel_active(d.doc.raw()) } {
        Ok(())
    } else {
        Err("nothing is selected; use select first".into())
    }
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ClipAction {
    Copy,
    Cut,
    Paste,
    PasteImage,
    Move,
    Commit,
    Cancel,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClipArgs {
    pub action: ClipAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer to copy from / paste into (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// paste/paste_image: left edge (default: the copied position, or 0).
    pub x: Option<i32>,
    /// paste/paste_image: top edge.
    pub y: Option<i32>,
    /// move: horizontal offset of the floating pixels.
    pub dx: Option<i32>,
    /// move: vertical offset.
    pub dy: Option<i32>,
    /// paste_image: PNG file to paste.
    pub path: Option<String>,
}

fn clipboard(e: &mut Engine, a: ClipArgs) -> R<Out> {
    let act = super::act(&a.action);
    let image = match a.action {
        ClipAction::PasteImage => {
            let p = e.sandbox.read(need(a.path.as_deref(), "path", &act)?)?;
            let bytes = std::fs::read(&p).map_err(|err| format!("{}: {err}", p.display()))?;
            Some(crate::image::decode_png(&bytes).map_err(|err| format!("{}: {err}", p.display()))?)
        }
        _ => None,
    };
    let clip_info = |c: &Clip| json!({ "x": c.x, "y": c.y, "w": c.w, "h": c.h });
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    match a.action {
        ClipAction::Copy | ClipAction::Cut => {
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            let mut b = [0i32; 4];
            unsafe { sel::pixmith_sel_copy_bounds(raw, b.as_mut_ptr()) };
            let (w, h) = (b[2].max(0) as u32, b[3].max(0) as u32);
            if w == 0 || h == 0 {
                return Err("nothing to copy".into());
            }
            let mut rgba = vec![0u8; w as usize * h as usize * 4];
            if !unsafe { sel::pixmith_sel_copy(raw, layer, frame, rgba.as_mut_ptr(), rgba.len()) } {
                return Err(fail("could not copy"));
            }
            if a.action == ClipAction::Cut && unsafe { sel::pixmith_sel_active(raw) } {
                d.edit("Cut", Undo::Step, |_| {
                    unsafe { sel::pixmith_sel_delete(raw, layer, frame) };
                    Ok(())
                })?;
            }
            let c = Clip { x: b[0], y: b[1], w, h, rgba };
            let info = clip_info(&c);
            let id = d.id.clone();
            e.clip = Some(c);
            Out::json(json!({ "doc": id, "copied": info }))
        }
        ClipAction::Paste | ClipAction::PasteImage => {
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            let (w, h, x, y, rgba) = match &image {
                Some(img) => (img.w as u32, img.h as u32, a.x.unwrap_or(0), a.y.unwrap_or(0), img.px.clone()),
                None => {
                    let c = e.clip.as_ref().ok_or("the clipboard is empty; copy first")?;
                    (c.w, c.h, a.x.unwrap_or(c.x), a.y.unwrap_or(c.y), c.rgba.clone())
                }
            };
            let d = e.doc(&a.doc)?;
            d.edit("Paste", Undo::Step, |_| {
                unsafe { sel::pixmith_sel_paste(raw, layer, frame, w, h, rgba.as_ptr(), x, y) }
                    .then_some(())
                    .ok_or_else(|| fail("could not paste"))
            })?;
            Out::json(
                json!({ "doc": d.id, "layer": layer, "frame": frame, "floating": { "x": x, "y": y, "w": w, "h": h },
                "next": "move it with action move, then commit (or cancel)" }),
            )
        }
        ClipAction::Move => {
            if !unsafe { sel::pixmith_float_active(raw) } {
                return Err("there is no floating paste; to move selected pixels use select action move_pixels".into());
            }
            let layer = d.image_layer(&a.layer)?;
            let frame = d.frame(a.frame)?;
            let (dx, dy) = (a.dx.unwrap_or(0), a.dy.unwrap_or(0));
            d.edit("Move", Undo::Live, |_| {
                unsafe {
                    if !sel::pixmith_sel_move_begin(raw, layer, frame) {
                        return Err(fail("could not move the floating pixels"));
                    }
                    sel::pixmith_sel_move_to(raw, dx, dy);
                    sel::pixmith_sel_move_end(raw);
                }
                Ok(())
            })?;
            Out::json(json!({ "doc": d.id, "selection": super::docs::selection_json(d) }))
        }
        ClipAction::Commit => {
            if !unsafe { sel::pixmith_float_active(raw) } {
                return Err("there is no floating paste to commit".into());
            }
            d.edit("Commit", Undo::Live, |_| {
                let _: () = unsafe { sel::pixmith_float_commit(raw) };
                Ok(())
            })?;
            Out::json(json!({ "doc": d.id, "committed": true, "selection": super::docs::selection_json(d),
                "note": "the pasted area stays selected; select none before editing outside it" }))
        }
        ClipAction::Cancel => {
            d.edit("Cancel", Undo::Live, |_| {
                unsafe { sel::pixmith_float_cancel(raw) }
                    .then_some(())
                    .ok_or_else(|| "there is no floating paste to cancel".to_string())
            })?;
            Out::json(json!({ "doc": d.id, "cancelled": true }))
        }
    }
}
