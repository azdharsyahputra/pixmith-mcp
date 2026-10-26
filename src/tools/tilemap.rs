//! Tilesets, tilemap layers, and terrain auto-tiling.

use super::view::Rect;
use super::{ADD, CHANGE, Out, ToolDef, need, tool};
use crate::color::{Color, hex};
use crate::doc::{cstr, fail, take_string};
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use crate::image::{Background, Img};
use pixmith_core::document::tilemap as tm;
use pixmith_core::gamedev::autotile as at;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub const TERRAIN_MODES: [&str; 3] = ["sides", "blob", "corners"];
const DIR_NAMES: [&str; 8] = ["n", "ne", "e", "se", "s", "sw", "w", "nw"];

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Tilemaps",
            "tileset",
            "Tilesets and tiles",
            CHANGE,
            TilesetArgs,
            tileset,
            "Work with tilesets (tile index 0 is always the empty tile). Actions: list, image (see every tile as one picture, labeled grid in index order), get_tile (pixels of one tile in the legend format), set_tile (replace a tile's pixels from `rows`+`legend` or a PNG `path`), add_tile (append an empty tile, or one with pixels), duplicate_tile, delete_tile (alias: remove_tile; cells using it become empty, higher indices shift down), rename, set_base_index (the number shown for tile 1). Drawing on a tilemap layer also edits the tiles it shows."
        ),
        tool!(
            "Tilemaps",
            "tilemap",
            "Tilemap layers",
            CHANGE,
            TilemapArgs,
            tilemap,
            "Tilemap layers: add_layer (new tilemap layer + tileset with tile_width x tile_height tiles, above `layer`), convert (turn an image layer into a tilemap, deduplicating tiles; flip_aware shares flipped copies), to_image (back to a normal layer), assign_tileset (use another tileset of the same tile size), stamp (the Tile Stamp mode: place `tile` at canvas pixel x, y, optional flip_x / flip_y / diagonal; tile 0 clears), get_tile, grid (the layer's tile indices as rows, read-only)."
        ),
        tool!(
            "Tilemaps",
            "terrain",
            "Auto-tile terrains",
            CHANGE,
            TerrainArgs,
            terrain,
            "Set up terrain auto-tiling on a tilemap layer (Godot-compatible peering). Modes: sides (16 tiles, N E S W), blob (47 tiles, sides + corners), corners (dual grid, 4 corners). Actions: create (first terrain + mode), set_mode, add / remove (alias: delete) / edit / move (terrain priority), list, generate_template (append a labeled template tileset for the mode; reduced = only flip/rotation representatives), assign_masks (give tiles 1.. the template's masks in order), set_tile (a tile's mask / terrain / peers / symmetry / weight), get_tile, set_options (edge_same, seed), rebuild (recompute every cell), clear_overrides, missing (cells that had no matching tile, read-only report). Paint with terrain_paint."
        ),
        tool!(
            "Tilemaps",
            "terrain_paint",
            "Terrain brush",
            ADD,
            PaintArgs,
            terrain_paint,
            "The terrain brush: paint terrain with the auto-tiling brush on a tilemap layer (the right tiles are chosen from the masks): paint (canvas pixel `points`, optional brush_size/round/mirror), rect (x, y, w, h), flood (fill the connected area at x, y), erase (paint terrain 0), override (force `tile` at x, y; 0 removes the override), pick (read the terrain at x, y). One undo step per call."
        ),
    ]
}

fn ts_info(d: &DocEntry, t: u32) -> Value {
    super::docs::tilesets_json(d).into_iter().nth(t as usize).unwrap_or(Value::Null)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TilesetAction {
    List,
    Image,
    GetTile,
    SetTile,
    AddTile,
    DuplicateTile,
    #[serde(alias = "remove_tile")]
    DeleteTile,
    Rename,
    SetBaseIndex,
}

#[derive(Deserialize, JsonSchema)]
pub struct TilesetArgs {
    pub action: TilesetAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Tileset index or name (default: the tileset of the active layer, else 0).
    pub tileset: Option<Ref>,
    /// get_tile/set_tile/duplicate_tile/delete_tile: tile index (1 = first real tile).
    pub tile: Option<u32>,
    /// rename: new name.
    pub name: Option<String>,
    /// set_base_index: number shown for tile 1.
    pub base_index: Option<i32>,
    /// set_tile/add_tile: one string per row, one legend character per pixel (as returned by get_tile).
    pub rows: Option<Vec<String>>,
    /// set_tile/add_tile: character -> color; "." is transparent unless mapped.
    pub legend: Option<BTreeMap<String, Color>>,
    /// set_tile/add_tile: PNG file with the tile's pixels (tile-sized).
    pub path: Option<String>,
    /// image: integer upscale (default auto).
    pub scale: Option<u32>,
}

fn default_tileset(d: &DocEntry, r: &Option<Ref>) -> R<u32> {
    match r {
        Some(r) => d.tileset(r),
        None => {
            let n = unsafe { tm::pixmith_tileset_count(d.doc.raw()) };
            if n == 0 {
                return Err(
                    "the document has no tilesets; create a tilemap layer first (tilemap action add_layer)".into()
                );
            }
            let t = unsafe { tm::pixmith_layer_tileset(d.doc.raw(), d.layer) };
            Ok(if t >= 0 { t as u32 } else { 0 })
        }
    }
}

fn tile_pixels(d: &DocEntry, ts: u32, tile: u32) -> R<(usize, usize, Vec<u8>)> {
    let raw = d.doc.raw();
    let (w, h) = unsafe {
        (tm::pixmith_tileset_tile_width(raw, ts) as usize, tm::pixmith_tileset_tile_height(raw, ts) as usize)
    };
    let mut px = vec![0u8; w * h * 4];
    if !unsafe { tm::pixmith_tileset_tile_pixels(raw, ts, tile, px.as_mut_ptr(), px.len()) } {
        return Err(format!("tile {tile} does not exist in tileset {ts}"));
    }
    Ok((w, h, px))
}

/// Tile pixels from rows + legend or a PNG, as a Lua table literal of colors.
fn pixels_lua(a: &TilesetArgs, w: usize, h: usize, png: Option<&std::path::Path>) -> R<String> {
    let colors: Vec<u32> = if let Some(p) = png {
        let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let img = crate::image::decode_png(&bytes)?;
        if (img.w, img.h) != (w, h) {
            return Err(format!("the PNG is {}x{}, the tiles are {w}x{h}", img.w, img.h));
        }
        img.px.chunks_exact(4).map(|p| u32::from_be_bytes([p[0], p[1], p[2], p[3]])).collect()
    } else {
        let rows = a.rows.as_ref().ok_or("give `rows` + `legend`, or `path`")?;
        let legend = a.legend.clone().unwrap_or_default();
        if rows.len() != h || rows.iter().any(|r| r.chars().count() != w) {
            return Err(format!("`rows` must be {h} strings of {w} characters (the tile size)"));
        }
        let mut out = Vec::with_capacity(w * h);
        for r in rows {
            for ch in r.chars() {
                let c = match legend.get(&ch.to_string()) {
                    Some(c) => c.0,
                    None if ch == '.' || ch == ' ' => 0,
                    None => return Err(format!("character {ch:?} is not in the legend")),
                };
                out.push(c);
            }
        }
        out
    };
    Ok(format!("{{{}}}", colors.iter().map(|c| format!("0x{c:08X}")).collect::<Vec<_>>().join(",")))
}

fn run_lua(d: &mut DocEntry, label: &str, src: &str) -> R<String> {
    let s = format!("app.undoLabel({label:?})\n{src}");
    super::script::run(d, &s)
}

fn tileset(e: &mut Engine, a: TilesetArgs) -> R<Out> {
    let png = match (&a.path, a.action) {
        (Some(p), TilesetAction::SetTile | TilesetAction::AddTile) => Some(e.sandbox.read(p)?),
        _ => None,
    };
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    if a.action == TilesetAction::List {
        return Out::json(json!({ "doc": d.id, "tilesets": super::docs::tilesets_json(d) }));
    }
    let ts = default_tileset(d, &a.tileset)?;
    let count = unsafe { tm::pixmith_tileset_tile_count(raw, ts) };
    let tile = |t: Option<u32>| -> R<u32> {
        let t = need(t, "tile", &act)?;
        if t >= count {
            Err(format!("tile {t} does not exist (tileset {ts} has tiles 0-{})", count.saturating_sub(1)))
        } else {
            Ok(t)
        }
    };
    let (tw, th) = unsafe {
        (tm::pixmith_tileset_tile_width(raw, ts) as usize, tm::pixmith_tileset_tile_height(raw, ts) as usize)
    };
    let mut extra = json!({});
    match a.action {
        TilesetAction::List => {}
        TilesetAction::Image => {
            let cols = (count as f64).sqrt().ceil().max(1.0) as usize;
            let rows = (count as usize).div_ceil(cols);
            let gap = 1;
            let mut sheet = Img::new(cols * (tw + gap) + gap, rows * (th + gap) + gap);
            for p in sheet.px.chunks_exact_mut(4) {
                p.copy_from_slice(&[0x40, 0x40, 0x48, 255]);
            }
            for t in 0..count as usize {
                let (_, _, px) = tile_pixels(d, ts, t as u32)?;
                let tile = Img { w: tw, h: th, px }.on_background(Background::Checker, 2);
                let (ox, oy) = (gap + (t % cols) * (tw + gap), gap + (t / cols) * (th + gap));
                for y in 0..th {
                    let dst = ((oy + y) * sheet.w + ox) * 4;
                    sheet.px[dst..dst + tw * 4].copy_from_slice(&tile.px[y * tw * 4..(y + 1) * tw * 4]);
                }
            }
            let scale = super::view::auto_scale(sheet.w, sheet.h, a.scale);
            let img = sheet.upscale(scale).png()?;
            return Ok(Out {
                value: json!({ "doc": d.id, "tileset": ts_info(d, ts), "layout": { "columns": cols, "rows": rows, "order": "row-major from tile 0 (top-left, always empty)", "cell": [tw + gap, th + gap], "scale": scale } }),
                images: vec![img],
                text: None,
            });
        }
        TilesetAction::GetTile => {
            let t = tile(a.tile)?;
            let (w, _, px) = tile_pixels(d, ts, t)?;
            let mut legend: BTreeMap<u32, char> = BTreeMap::new();
            let chars: Vec<char> = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars().collect();
            let mut rows = Vec::new();
            for row in px.chunks_exact(w * 4) {
                let mut s = String::new();
                for p in row.chunks_exact(4) {
                    let c = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
                    if c & 0xFF == 0 {
                        s.push('.');
                        continue;
                    }
                    let n = legend.len();
                    let ch = *legend.entry(c).or_insert_with(|| chars.get(n).copied().unwrap_or('?'));
                    s.push(ch);
                }
                rows.push(s);
            }
            let lg: BTreeMap<String, String> = legend.iter().map(|(c, ch)| (ch.to_string(), hex(*c))).collect();
            extra = json!({ "tile": t, "width": tw, "height": th, "rows": rows, "legend": lg });
        }
        TilesetAction::SetTile => {
            let t = tile(a.tile)?;
            if t == 0 {
                return Err("tile 0 is the empty tile and cannot be changed".into());
            }
            let px = pixels_lua(&a, tw, th, png.as_deref())?;
            run_lua(d, "Edit Tile", &format!("sprite.tilesets[{}]:setTilePixels({t}, {px})", ts + 1))?;
            extra = json!({ "tile": t });
        }
        TilesetAction::AddTile => {
            let t = if a.rows.is_some() || png.is_some() {
                let px = pixels_lua(&a, tw, th, png.as_deref())?;
                run_lua(d, "Add Tile", &format!("print(sprite.tilesets[{}]:addTile({px}))", ts + 1))?
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| "could not add the tile".to_string())?
            } else {
                d.edit("Add Tile", Undo::Step, |_| {
                    let t = unsafe { tm::pixmith_tileset_add_tile(raw, ts) };
                    if t == 0 { Err(fail("could not add a tile")) } else { Ok(t) }
                })?
            };
            extra = json!({ "tile": t });
        }
        TilesetAction::DuplicateTile => {
            let t = tile(a.tile)?;
            let n = d.edit("Duplicate Tile", Undo::Step, |_| {
                let n = unsafe { tm::pixmith_tileset_duplicate_tile(raw, ts, t) };
                if n == 0 { Err(fail("could not duplicate the tile")) } else { Ok(n) }
            })?;
            extra = json!({ "tile": n });
        }
        TilesetAction::DeleteTile => {
            let t = tile(a.tile)?;
            d.edit("Delete Tile", Undo::Step, |_| {
                unsafe { tm::pixmith_tileset_delete_tile(raw, ts, t) }
                    .then_some(())
                    .ok_or_else(|| fail("cannot delete that tile"))
            })?;
        }
        TilesetAction::Rename => {
            let c = cstr(need(a.name.as_deref(), "name", &act)?);
            d.edit("Rename Tileset", Undo::Step, |_| {
                unsafe { tm::pixmith_tileset_rename(raw, ts, c.as_ptr()) }
                    .then_some(())
                    .ok_or_else(|| fail("could not rename"))
            })?;
        }
        TilesetAction::SetBaseIndex => {
            let b = need(a.base_index, "base_index", &act)?;
            d.edit("Tileset Base Index", Undo::Step, |_| {
                unsafe { tm::pixmith_tileset_set_base_index(raw, ts, b) };
                Ok(())
            })?;
        }
    }
    let mut v = json!({ "doc": d.id, "tileset": ts_info(d, ts) });
    if let (Some(o), Some(x)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    Out::json(v)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TilemapAction {
    AddLayer,
    Convert,
    ToImage,
    AssignTileset,
    Stamp,
    GetTile,
    Grid,
}

#[derive(Deserialize, JsonSchema)]
pub struct TilemapArgs {
    pub action: TilemapAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// The tilemap layer (default: active). add_layer: the new layer goes above it.
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// add_layer/convert: tile width in pixels.
    pub tile_width: Option<u32>,
    /// add_layer/convert: tile height (default = tile_width).
    pub tile_height: Option<u32>,
    /// convert: flipped/rotated copies share one tile.
    pub flip_aware: Option<bool>,
    /// assign_tileset: tileset index or name.
    pub tileset: Option<Ref>,
    /// stamp/get_tile: canvas pixel x inside the cell.
    pub x: Option<i32>,
    /// stamp/get_tile: canvas pixel y inside the cell.
    pub y: Option<i32>,
    /// stamp: tile index (0 clears the cell).
    pub tile: Option<u32>,
    pub flip_x: Option<bool>,
    pub flip_y: Option<bool>,
    /// Diagonal flip (transpose; square tiles only).
    pub diagonal: Option<bool>,
    /// grid: only these canvas pixels.
    pub region: Option<Rect>,
    /// add_layer: name of the new layer.
    pub name: Option<String>,
}

fn decode_tile(v: u32) -> Value {
    let idx = v & 0x1FFF_FFFF;
    json!({ "tile": idx, "flip_x": v & 0x8000_0000 != 0, "flip_y": v & 0x4000_0000 != 0, "diagonal": v & 0x2000_0000 != 0 })
}

fn tilemap(e: &mut Engine, a: TilemapArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let f = d.frame(a.frame)?;
    let mut out = json!({ "doc": d.id });
    match a.action {
        TilemapAction::AddLayer => {
            let above = d.layer(&a.layer)?;
            let tw = need(a.tile_width, "tile_width", &act)?.clamp(1, 1024);
            let th = a.tile_height.unwrap_or(tw).clamp(1, 1024);
            let name = a.name.clone();
            let l = d.edit("New Tilemap Layer", Undo::Step, |_| {
                let l = unsafe { tm::pixmith_add_tilemap_layer(raw, above, tw, th) };
                if let Some(n) = &name {
                    let c = cstr(n);
                    unsafe { pixmith_core::ffi::document::pixmith_rename_layer(raw, l, c.as_ptr()) };
                }
                Ok(l)
            })?;
            d.layer = l;
            out["layer"] = json!(l);
            out["tileset"] = json!(unsafe { tm::pixmith_layer_tileset(raw, l) });
        }
        TilemapAction::Convert => {
            let l = d.image_layer(&a.layer)?;
            let tw = need(a.tile_width, "tile_width", &act)?.clamp(1, 1024);
            let th = a.tile_height.unwrap_or(tw).clamp(1, 1024);
            let fa = a.flip_aware.unwrap_or(false);
            d.edit("Convert to Tilemap", Undo::Step, |_| {
                unsafe { tm::pixmith_convert_to_tilemap_ex(raw, l, tw, th, fa) }
                    .then_some(())
                    .ok_or_else(|| fail("could not convert the layer"))
            })?;
            out["layer"] = json!(l);
            out["tileset"] = json!(unsafe { tm::pixmith_layer_tileset(raw, l) });
        }
        TilemapAction::ToImage => {
            let l = d.tilemap_layer(&a.layer)?;
            d.edit("Convert to Normal Layer", Undo::Step, |_| {
                unsafe { tm::pixmith_convert_to_normal(raw, l) }.then_some(()).ok_or_else(|| fail("could not convert"))
            })?;
            out["layer"] = json!(l);
        }
        TilemapAction::AssignTileset => {
            let l = d.tilemap_layer(&a.layer)?;
            let ts = d.tileset(&need(a.tileset.clone(), "tileset", &act)?)?;
            d.edit("Change Tileset", Undo::Step, |_| {
                unsafe { tm::pixmith_layer_assign_tileset(raw, l, ts) }
                    .then_some(())
                    .ok_or_else(|| fail("the tileset's tile size must match the layer's"))
            })?;
            out["layer"] = json!(l);
        }
        TilemapAction::Stamp => {
            let l = d.tilemap_layer(&a.layer)?;
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let t = need(a.tile, "tile", &act)?;
            let ts = unsafe { tm::pixmith_layer_tileset(raw, l) }.max(0) as u32;
            let count = unsafe { tm::pixmith_tileset_tile_count(raw, ts) };
            if t >= count {
                return Err(format!("tile {t} does not exist (tileset {ts} has tiles 0-{})", count.saturating_sub(1)));
            }
            let v = t
                | if a.flip_x.unwrap_or(false) { 0x8000_0000 } else { 0 }
                | if a.flip_y.unwrap_or(false) { 0x4000_0000 } else { 0 }
                | if a.diagonal.unwrap_or(false) { 0x2000_0000 } else { 0 };
            d.edit("Stamp Tile", Undo::Step, |_| {
                unsafe { tm::pixmith_tile_stamp(raw, l, f, x, y, v) }
                    .then_some(())
                    .ok_or_else(|| fail(format!("could not stamp at ({x}, {y}) (outside the map?)")))
            })?;
            out["cell"] = decode_tile(unsafe { tm::pixmith_tile_at(raw, l, f, x, y) });
        }
        TilemapAction::GetTile => {
            let l = d.tilemap_layer(&a.layer)?;
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            out["cell"] = decode_tile(unsafe { tm::pixmith_tile_at(raw, l, f, x, y) });
        }
        TilemapAction::Grid => {
            let l = d.tilemap_layer(&a.layer)?;
            let ts = unsafe { tm::pixmith_layer_tileset(raw, l) }.max(0) as u32;
            let (tw, th) = unsafe {
                (tm::pixmith_tileset_tile_width(raw, ts) as i32, tm::pixmith_tileset_tile_height(raw, ts) as i32)
            };
            let mut o = [0i32; 2];
            unsafe { tm::pixmith_tile_grid_origin(raw, l, f, o.as_mut_ptr()) };
            let (x0, y0, w, h) = super::view::clip(a.region, d.doc.width(), d.doc.height())?;
            // Cells whose top-left lies in the region, on the layer's grid.
            let first =
                |p: i32, origin: i32, size: i32| origin + ((p - origin) as f64 / size as f64).floor() as i32 * size;
            let (cx0, cy0) = (
                first(x0 as i32, o[0] - if o[0] > 0 { tw } else { 0 }, tw),
                first(y0 as i32, o[1] - if o[1] > 0 { th } else { 0 }, th),
            );
            let mut rows = Vec::new();
            let mut cy = cy0;
            while cy < (y0 + h) as i32 && rows.len() < 256 {
                let mut row = Vec::new();
                let mut cx = cx0;
                while cx < (x0 + w) as i32 && row.len() < 256 {
                    let v = unsafe { tm::pixmith_tile_at(raw, l, f, cx.max(0), cy.max(0)) };
                    let flips = if v & 0xE000_0000 != 0 { "*" } else { "" };
                    row.push(format!("{}{flips}", v & 0x1FFF_FFFF));
                    cx += tw;
                }
                rows.push(row.join(" "));
                cy += th;
            }
            out["layer"] = json!(l);
            out["tile_size"] = json!([tw, th]);
            out["grid_origin"] = json!(o);
            out["first_cell_pixel"] = json!([cx0, cy0]);
            out["rows"] = json!(rows);
            out["legend"] = json!("tile index per cell, 0 = empty, * = flipped");
        }
    }
    Out::json(out)
}

// MARK: Terrains

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TerrainMode {
    Sides,
    Blob,
    Corners,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TerrainAction {
    Create,
    SetMode,
    Add,
    #[serde(alias = "delete")]
    Remove,
    Edit,
    Move,
    List,
    GenerateTemplate,
    AssignMasks,
    SetTile,
    GetTile,
    SetOptions,
    Rebuild,
    ClearOverrides,
    Missing,
}

#[derive(Deserialize, JsonSchema)]
pub struct TerrainArgs {
    pub action: TerrainAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Tilemap layer (default: active).
    pub layer: Option<Ref>,
    /// Frame for clear_overrides/missing (default: active).
    pub frame: Option<u32>,
    /// create/set_mode: auto-tiling mode.
    pub mode: Option<TerrainMode>,
    /// create/add/edit: terrain name.
    pub name: Option<String>,
    /// create/add/edit: terrain color (for display and Godot).
    pub color: Option<Color>,
    /// remove/edit/move/set_tile: terrain id (1-15) or name.
    pub terrain: Option<Ref>,
    /// move: new priority position (0 = highest).
    pub to: Option<u32>,
    /// generate_template: only flip/rotation representatives.
    pub reduced: Option<bool>,
    /// set_tile/get_tile: tile index.
    pub tile: Option<u32>,
    /// set_tile: neighbor mask (sides: N=1 E=2 S=4 W=8; blob: N=1 NE=2 E=4 SE=8 S=16 SW=32 W=64 NW=128; corners: TL=1 TR=2 BR=4 BL=8); -1 removes the tile's terrain data.
    pub mask: Option<i32>,
    /// set_tile: terrain of each ring direction, e.g. {"n": 1, "e": 0} (n ne e se s sw w nw; 0 = empty).
    pub peers: Option<BTreeMap<String, u32>>,
    /// set_tile: allowed symmetries bitmask (1 flip X, 2 flip Y, 4 diagonal).
    pub symmetry: Option<u32>,
    /// set_tile: random weight among tiles with the same mask.
    pub weight: Option<f32>,
    /// set_options: cells outside the map count as the same terrain.
    pub edge_same: Option<bool>,
    /// set_options: random seed for weighted variants.
    pub seed: Option<u32>,
}

fn terrain_ids(d: &DocEntry, l: u32) -> Vec<u32> {
    let raw = d.doc.raw();
    let n = unsafe { at::pixmith_autotile_terrain_count(raw, l) };
    (0..n).map(|i| unsafe { at::pixmith_autotile_terrain_id(raw, l, i) }).collect()
}

fn terrain_name(d: &DocEntry, l: u32, id: u32) -> String {
    unsafe { take_string(at::pixmith_autotile_terrain_name(d.doc.raw(), l, id)) }
}

fn terrain_id(d: &DocEntry, l: u32, r: &Ref) -> R<u32> {
    let ids = terrain_ids(d, l);
    match r {
        Ref::Index(0) => Ok(0),
        Ref::Index(i) if ids.contains(i) => Ok(*i),
        Ref::Index(i) => Err(format!("terrain id {i} does not exist (ids: {ids:?}; 0 = none)")),
        Ref::Name(n) => {
            if let Ok(i) = n.parse::<u32>() {
                return terrain_id(d, l, &Ref::Index(i));
            }
            ids.iter()
                .copied()
                .find(|&id| terrain_name(d, l, id).eq_ignore_ascii_case(n))
                .ok_or_else(|| format!("no terrain named {n:?} on layer {l}"))
        }
    }
}

fn terrains_json(d: &DocEntry, l: u32) -> Value {
    let raw = d.doc.raw();
    let mode = unsafe { at::pixmith_autotile_mode(raw, l) };
    let list: Vec<Value> = terrain_ids(d, l)
        .iter()
        .enumerate()
        .map(|(p, &id)| {
            json!({ "id": id, "priority": p, "name": terrain_name(d, l, id), "color": hex(unsafe { at::pixmith_autotile_terrain_color(raw, l, id) }) })
        })
        .collect();
    json!({
        "layer": l,
        "mode": if mode >= 0 { json!(TERRAIN_MODES[mode as usize % 3]) } else { Value::Null },
        "terrains": list,
        "edge_same": unsafe { at::pixmith_autotile_edge_same(raw, l) },
        "seed": unsafe { at::pixmith_autotile_seed(raw, l) },
    })
}

fn tile_terrain_json(d: &DocEntry, l: u32, t: u32) -> Value {
    let raw = d.doc.raw();
    let peers: BTreeMap<&str, i32> = DIR_NAMES
        .iter()
        .enumerate()
        .map(|(i, n)| (*n, unsafe { at::pixmith_autotile_tile_peer(raw, l, t, i as u32) }))
        .collect();
    json!({
        "tile": t,
        "mask": unsafe { at::pixmith_autotile_tile_mask(raw, l, t) },
        "terrain": unsafe { at::pixmith_autotile_tile_terrain(raw, l, t) },
        "peers": peers,
        "symmetry": unsafe { at::pixmith_autotile_tile_sym(raw, l, t) },
        "weight": unsafe { at::pixmith_autotile_tile_weight(raw, l, t) },
    })
}

fn terrain(e: &mut Engine, a: TerrainArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let l = d.tilemap_layer(&a.layer)?;
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(fail(what.to_string())) };
    let has_data = unsafe { at::pixmith_autotile_mode(raw, l) } >= 0;
    let requires =
        || if has_data { Ok(()) } else { Err("this layer has no terrains yet; use action create first".to_string()) };
    let mut extra = Value::Null;
    match a.action {
        TerrainAction::Create => {
            let mode = need(a.mode, "mode", &act)? as i32;
            let name = cstr(a.name.as_deref().unwrap_or("Terrain"));
            let color = a.color.map_or(0x4CB04CFF, |c| c.0);
            d.edit("New Terrain", Undo::Step, |_| {
                ok(
                    unsafe { at::pixmith_autotile_set_terrain(raw, l, name.as_ptr(), color, mode) },
                    "could not create the terrain",
                )
            })?;
        }
        TerrainAction::SetMode => {
            let mode = need(a.mode, "mode", &act)? as i32;
            requires()?;
            d.edit("Terrain Mode", Undo::Step, |_| {
                ok(unsafe { at::pixmith_autotile_set_mode(raw, l, mode) }, "could not set the mode")
            })?;
        }
        TerrainAction::Add => {
            requires()?;
            let name = cstr(need(a.name.as_deref(), "name", &act)?);
            let color = a.color.map_or(0x8060C0FF, |c| c.0);
            let id = d.edit("Add Terrain", Undo::Step, |_| {
                let id = unsafe { at::pixmith_autotile_add_terrain(raw, l, name.as_ptr(), color) };
                if id == 0 { Err(fail("could not add a terrain (at most 15)")) } else { Ok(id) }
            })?;
            extra = json!({ "added": id });
        }
        TerrainAction::Remove | TerrainAction::Edit | TerrainAction::Move => {
            requires()?;
            let id = terrain_id(d, l, &need(a.terrain.clone(), "terrain", &act)?)?;
            match a.action {
                TerrainAction::Remove => d.edit("Remove Terrain", Undo::Step, |_| {
                    ok(unsafe { at::pixmith_autotile_remove_terrain(raw, l, id) }, "could not remove")
                })?,
                TerrainAction::Edit => {
                    let name = cstr(&a.name.clone().unwrap_or_else(|| terrain_name(d, l, id)));
                    let color = a.color.map_or(unsafe { at::pixmith_autotile_terrain_color(raw, l, id) }, |c| c.0);
                    d.edit("Edit Terrain", Undo::Step, |_| {
                        ok(
                            unsafe { at::pixmith_autotile_edit_terrain(raw, l, id, name.as_ptr(), color) },
                            "could not edit",
                        )
                    })?
                }
                _ => {
                    let to = need(a.to, "to", &act)?;
                    d.edit("Terrain Priority", Undo::Step, |_| {
                        ok(unsafe { at::pixmith_autotile_move_terrain(raw, l, id, to) }, "could not move")
                    })?
                }
            }
        }
        TerrainAction::List => {}
        TerrainAction::GenerateTemplate => {
            requires()?;
            let reduced = a.reduced.unwrap_or(false);
            let n = d.edit("Generate Terrain Template", Undo::Step, |_| {
                let n = unsafe { at::pixmith_autotile_generate_template(raw, l, reduced) };
                if n == 0 { Err(fail("could not generate the template")) } else { Ok(n) }
            })?;
            extra = json!({ "tiles_added": n, "hint": "the template tiles are labeled outlines; draw over them (tileset set_tile, or draw on the layer) and keep their order" });
        }
        TerrainAction::AssignMasks => {
            requires()?;
            let n = d.edit("Assign Template Masks", Undo::Step, |_| {
                Ok(unsafe { at::pixmith_autotile_assign_masks_from_template(raw, l) })
            })?;
            extra = json!({ "tiles_assigned": n });
        }
        TerrainAction::SetTile => {
            requires()?;
            let t = need(a.tile, "tile", &act)?;
            let tid = a.terrain.as_ref().map(|r| terrain_id(d, l, r)).transpose()?;
            let mut peers = Vec::new();
            for (k, v) in a.peers.iter().flatten() {
                let i = DIR_NAMES
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(k))
                    .ok_or_else(|| format!("unknown direction {k:?}; use n ne e se s sw w nw"))?;
                peers.push((i as u32, *v));
            }
            d.edit("Tile Terrain", Undo::Step, |_| {
                unsafe {
                    if let Some(m) = a.mask {
                        ok(at::pixmith_autotile_set_tile_mask(raw, l, t, m), "could not set the mask")?;
                    }
                    if let Some(id) = tid {
                        ok(at::pixmith_autotile_set_tile_terrain(raw, l, t, id), "could not set the terrain")?;
                    }
                    for &(dir, v) in &peers {
                        ok(at::pixmith_autotile_set_tile_peer(raw, l, t, dir, v), "could not set a peer")?;
                    }
                    if let Some(s) = a.symmetry {
                        ok(at::pixmith_autotile_set_tile_sym(raw, l, t, s), "could not set the symmetry")?;
                    }
                    if let Some(w) = a.weight {
                        ok(at::pixmith_autotile_set_tile_weight(raw, l, t, w), "could not set the weight")?;
                    }
                }
                Ok(())
            })?;
            extra = tile_terrain_json(d, l, t);
        }
        TerrainAction::GetTile => {
            extra = tile_terrain_json(d, l, need(a.tile, "tile", &act)?);
        }
        TerrainAction::SetOptions => {
            requires()?;
            d.edit("Terrain Options", Undo::Step, |_| {
                if let Some(s) = a.edge_same {
                    ok(unsafe { at::pixmith_autotile_set_edge_same(raw, l, s) }, "could not set edge_same")?;
                }
                if let Some(s) = a.seed {
                    ok(unsafe { at::pixmith_autotile_set_seed(raw, l, s) }, "could not set the seed")?;
                }
                Ok(())
            })?;
        }
        TerrainAction::Rebuild => {
            requires()?;
            d.edit("Rebuild Terrain", Undo::Step, |_| {
                ok(unsafe { at::pixmith_autotile_rebuild(raw, l) }, "could not rebuild")
            })?;
        }
        TerrainAction::ClearOverrides => {
            let f = d.frame(a.frame)?;
            d.edit_changed("Clear Overrides", |_| Ok(unsafe { at::pixmith_autotile_clear_overrides(raw, l, f) }))?;
        }
        TerrainAction::Missing => {
            let f = d.frame(a.frame)?;
            let n = unsafe { at::pixmith_autotile_missing_cells(raw, l, f, std::ptr::null_mut(), 0) };
            let cap = n.min(1024);
            let mut cells = vec![0i32; cap * 2];
            unsafe { at::pixmith_autotile_missing_cells(raw, l, f, cells.as_mut_ptr(), cap) };
            let report = unsafe { take_string(at::pixmith_autotile_missing_report(raw, l, f)) };
            extra = json!({
                "missing_cells": n,
                "cells_top_left_pixels": cells.chunks_exact(2).map(|c| [c[0], c[1]]).collect::<Vec<_>>(),
                "report": report.lines().take(200).collect::<Vec<_>>(),
            });
        }
    }
    let mut v = terrains_json(d, l);
    v["doc"] = json!(d.id);
    if !extra.is_null() {
        v["result"] = extra;
    }
    Out::json(v)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PaintAction {
    Paint,
    Erase,
    Rect,
    Flood,
    Override,
    Pick,
}

#[derive(Deserialize, JsonSchema)]
pub struct PaintArgs {
    pub action: PaintAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Tilemap layer (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// Terrain id (1-15) or name to paint (paint/rect/flood).
    pub terrain: Option<Ref>,
    /// paint/erase: canvas pixel positions [[x, y], ...].
    pub points: Option<Vec<[i32; 2]>>,
    /// rect/flood/override/pick: canvas pixel x.
    pub x: Option<i32>,
    /// rect/flood/override/pick: canvas pixel y.
    pub y: Option<i32>,
    /// rect: width in pixels.
    pub w: Option<u32>,
    /// rect: height in pixels.
    pub h: Option<u32>,
    /// paint/erase: brush size in cells (default 1).
    pub brush_size: Option<u32>,
    /// paint/erase: round brush.
    pub round: Option<bool>,
    /// Symmetry: none, horizontal, vertical or both.
    pub mirror: Option<super::draw::Mirror>,
    /// override: tile index to force (0 removes the override).
    pub tile: Option<u32>,
}

fn terrain_paint(e: &mut Engine, a: PaintArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let l = d.tilemap_layer(&a.layer)?;
    let f = d.frame(a.frame)?;
    if unsafe { at::pixmith_autotile_mode(raw, l) } < 0 {
        return Err("this layer has no terrains; set them up with the terrain tool (action create)".into());
    }
    let mirror = a.mirror.unwrap_or_default().flags();
    let terrain = |d: &DocEntry| -> R<u32> {
        if a.action == PaintAction::Erase {
            return Ok(0);
        }
        terrain_id(d, l, &need(a.terrain.clone(), "terrain", &act)?)
    };
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(fail(what.to_string())) };
    let mut extra = json!({});
    match a.action {
        PaintAction::Paint | PaintAction::Erase => {
            let t = terrain(d)?;
            let pts = need(a.points.as_ref(), "points", &act)?;
            let (size, round) = (a.brush_size.unwrap_or(1).clamp(1, 64), a.round.unwrap_or(false));
            d.edit(if t == 0 { "Erase Terrain" } else { "Paint Terrain" }, Undo::Step, |_| {
                for p in pts {
                    unsafe { at::pixmith_autotile_paint_brush(raw, l, f, p[0], p[1], t, size, round, mirror) };
                }
                Ok(())
            })?;
        }
        PaintAction::Rect => {
            let t = terrain(d)?;
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let (w, h) = (need(a.w, "w", &act)?.max(1) as i32, need(a.h, "h", &act)?.max(1) as i32);
            d.edit("Terrain Rectangle", Undo::Step, |_| {
                ok(
                    unsafe { at::pixmith_autotile_rect(raw, l, f, x, y, x + w - 1, y + h - 1, t, mirror) },
                    "could not paint the rectangle",
                )
            })?;
        }
        PaintAction::Flood => {
            let t = terrain(d)?;
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            d.edit("Terrain Fill", Undo::Step, |_| {
                ok(unsafe { at::pixmith_autotile_flood(raw, l, f, x, y, t, mirror) }, "could not fill")
            })?;
        }
        PaintAction::Override => {
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let t = need(a.tile, "tile", &act)?;
            d.edit("Tile Override", Undo::Step, |_| {
                ok(unsafe { at::pixmith_autotile_set_override(raw, l, f, x, y, t) }, "could not set the override")
            })?;
        }
        PaintAction::Pick => {
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            let id = unsafe { at::pixmith_autotile_terrain_at(raw, l, f, x, y) };
            extra = json!({ "terrain": id, "name": (id > 0).then(|| terrain_name(d, l, id)) });
        }
    }
    let n = unsafe { at::pixmith_autotile_missing_cells(raw, l, f, std::ptr::null_mut(), 0) };
    let mut v = json!({ "doc": d.id, "layer": l, "frame": f, "missing_tile_cells": n });
    if let (Some(o), Some(x)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(x.clone());
    }
    if n > 0 {
        v["hint"] = json!("some cells found no tile with a matching mask; see terrain action missing");
    }
    Out::json(v)
}
