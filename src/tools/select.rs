//! Selection and clipboard. The selection is part of the document (and of its undo history); drawing,
//! filters, gradients and fills are clipped to it.

use super::{ADD, CHANGE, Out, ToolDef, need, tool};
use crate::doc::fail;
use crate::engine::{Clip, DocEntry, Engine, R, Ref, Undo};
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
            "Change the selection (one undo step each): rect (x, y, w, h), polygon/lasso (points), wand (magic wand at x, y with tolerance; contiguous by default), by_color (every pixel matching the color at x, y), all, none, invert, expand/contract/border (by `amount` pixels). mode = replace (default), add or subtract combines with the existing selection. Also edits the selected pixels: delete (clear them), move_pixels (shift them by dx, dy), crop (crop the canvas to the selection bounds). Returns the selection bounds. Drawing and filters only affect selected pixels while a selection exists; select none to remove it."
        ),
        tool!(
            "Selection",
            "clipboard",
            "Copy / cut / paste",
            ADD,
            ClipArgs,
            clipboard,
            "Clipboard (shared by all documents): copy or cut the selected pixels of a layer (the whole canvas when nothing is selected); paste them (or a PNG file with paste_image) as a floating selection at (x, y) (default: where they were copied from); move the floating pixels by (dx, dy); commit to drop them onto the layer, or cancel to discard the paste. A floating paste is also committed by the next edit."
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
    /// replace (default), add or subtract — for rect, polygon, lasso, wand, by_color.
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
    /// Layer sampled by wand/by_color and edited by delete/move_pixels (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
}

fn select(e: &mut Engine, a: SelectArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let act = super::act(&a.action);
    let mode = a.mode.unwrap_or_default() as u32;
    let raw = d.doc.raw();
    let label = match a.action {
        SelectAction::Rect => "Rectangle Selection",
        SelectAction::Polygon | SelectAction::Lasso => "Lasso Selection",
        SelectAction::Wand => "Magic Wand",
        SelectAction::ByColor => "Select Color",
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
    match a.action {
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
