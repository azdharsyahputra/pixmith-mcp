//! Transforms: flips and rotations of cels or the selection, free rotation with RotSprite, the free-transform
//! session, sprite scaling and canvas size.

use super::{CHANGE, Out, ToolDef, need, tool};
use crate::doc::fail;
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use pixmith_core::paint::{filters, selection as sel, transform as xf};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Transform",
            "transform",
            "Flip / rotate",
            CHANGE,
            TransformArgs,
            transform,
            "Flip or rotate the selected pixels, or the whole cel when nothing is selected: flip_horizontal, flip_vertical, rotate_cw / rotate_ccw / rotate_180 (exact 90-degree turns), or rotate by any `angle` in degrees (clockwise) with RotSprite resampling (keeps pixel-art edges crisp; rotsprite=false for nearest neighbour). all_frames / all_layers repeat it over frames / layers, all in one undo step."
        ),
        tool!(
            "Transform",
            "free_transform",
            "Free transform session",
            CHANGE,
            XfArgs,
            free_transform,
            "Interactive free transform of the selection (or cel), like Cmd-T in the app: begin, then set any of x, y, width, height, angle (degrees), skew_x, skew_y (the preview updates; `anchor` picks the fixed point for width/height), read the state with info, and commit (one undo step) or cancel. Any other edit also commits it."
        ),
        tool!(
            "Transform",
            "sprite_size",
            "Scale sprite / canvas size",
            CHANGE,
            SizeArgs,
            sprite_size,
            "Resize the whole sprite (all layers and frames): `scale` resamples the pixels (nearest neighbour) to width x height (or by `factor`); `canvas` changes the canvas size without scaling, placing the old image by `anchor` (default center) or explicit offset_x/offset_y (where the old top-left lands; negative crops)."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TransformAction {
    FlipHorizontal,
    FlipVertical,
    RotateCw,
    RotateCcw,
    #[serde(rename = "rotate_180")]
    Rotate180,
    Rotate,
}

#[derive(Deserialize, JsonSchema)]
pub struct TransformArgs {
    pub action: TransformAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// Apply to every frame of the layer.
    pub all_frames: Option<bool>,
    /// Apply to every image layer of the frame(s).
    pub all_layers: Option<bool>,
    /// rotate: angle in degrees, clockwise.
    pub angle: Option<f64>,
    /// rotate: RotSprite resampling (default true).
    pub rotsprite: Option<bool>,
}

fn targets(
    d: &DocEntry,
    layer: &Option<Ref>,
    frame: Option<u32>,
    all_layers: bool,
    all_frames: bool,
) -> R<Vec<(u32, u32)>> {
    let layers: Vec<u32> = if all_layers {
        (0..d.layer_count()).filter(|&l| !d.is_group(l)).collect()
    } else {
        vec![d.image_layer(layer)?]
    };
    let frames: Vec<u32> = if all_frames { (0..d.doc.frame_count()).collect() } else { vec![d.frame(frame)?] };
    Ok(layers.iter().flat_map(|&l| frames.iter().map(move |&f| (l, f))).collect())
}

fn transform(e: &mut Engine, a: TransformArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let cels = targets(d, &a.layer, a.frame, a.all_layers.unwrap_or(false), a.all_frames.unwrap_or(false))?;
    let raw = d.doc.raw();
    let ops: Vec<u32> = match a.action {
        TransformAction::FlipHorizontal => vec![0],
        TransformAction::FlipVertical => vec![1],
        TransformAction::RotateCw => vec![2],
        TransformAction::RotateCcw => vec![3],
        TransformAction::Rotate180 => vec![2, 2],
        TransformAction::Rotate => vec![],
    };
    let label = match a.action {
        TransformAction::FlipHorizontal => "Flip Horizontal",
        TransformAction::FlipVertical => "Flip Vertical",
        TransformAction::Rotate => "Rotate",
        _ => "Rotate 90",
    };
    let linked_seen = std::cell::RefCell::new(Vec::new());
    if a.action == TransformAction::Rotate {
        let angle = need(a.angle, "angle", "rotate")?;
        let rs = a.rotsprite.unwrap_or(true);
        d.doc.begin_transaction();
        let r = (|| {
            for &(l, f) in &cels {
                if skip_linked(d, l, f, &linked_seen) {
                    continue;
                }
                unsafe {
                    if !xf::pixmith_xf_begin(raw, l, f, rs) {
                        return Err(fail(format!("could not transform layer {l} frame {f}")));
                    }
                    xf::pixmith_xf_set_field(raw, 4, angle);
                    xf::pixmith_xf_commit(raw);
                }
            }
            Ok(())
        })();
        d.doc.end_transaction();
        r?;
        d.doc.label(label);
        d.doc.touch();
    } else {
        d.edit(label, Undo::Step, |d| {
            for &(l, f) in &cels {
                if skip_linked(d, l, f, &linked_seen) {
                    continue;
                }
                for &op in &ops {
                    if !unsafe { sel::pixmith_sel_transform(raw, l, f, op) } {
                        return Err(fail(format!("could not transform layer {l} frame {f}")));
                    }
                }
            }
            Ok(())
        })?;
    }
    Out::json(json!({ "doc": d.id, "cels": cels.len(), "selection": super::docs::selection_json(d) }))
}

/// Linked cels share one image: transform it once.
fn skip_linked(d: &DocEntry, l: u32, f: u32, seen: &std::cell::RefCell<Vec<usize>>) -> bool {
    // Unlinked copies may share a buffer copy-on-write too; only real links count.
    if !unsafe { pixmith_core::document::layers::pixmith_cel_is_linked(d.doc.raw(), l, f) } {
        return false;
    }
    let s = d.doc.sprite();
    let Some(Some(c)) = s.cels.get(f as usize).and_then(|r| r.get(l as usize)) else { return false };
    let key = std::sync::Arc::as_ptr(c) as usize;
    let mut seen = seen.borrow_mut();
    if seen.contains(&key) {
        return true;
    }
    seen.push(key);
    false
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum XfAction {
    Begin,
    Set,
    Info,
    Commit,
    Cancel,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl Anchor {
    /// (-1..1, -1..1)
    pub fn dir(self) -> (i32, i32) {
        let i = self as i32;
        (i % 3 - 1, i / 3 - 1)
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct XfArgs {
    pub action: XfAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// begin: layer (default: active).
    pub layer: Option<Ref>,
    /// begin: frame (default: active).
    pub frame: Option<u32>,
    /// begin/set: RotSprite resampling for rotation (default true).
    pub rotsprite: Option<bool>,
    /// set: left of the box.
    pub x: Option<f64>,
    /// set: top of the box.
    pub y: Option<f64>,
    /// set: box width.
    pub width: Option<f64>,
    /// set: box height.
    pub height: Option<f64>,
    /// set: rotation in degrees, clockwise.
    pub angle: Option<f64>,
    /// set: horizontal skew in degrees.
    pub skew_x: Option<f64>,
    /// set: vertical skew in degrees.
    pub skew_y: Option<f64>,
    /// set: the point that stays fixed when width/height change (default center).
    pub anchor: Option<Anchor>,
}

fn xf_state(d: &DocEntry) -> Value {
    let raw = d.doc.raw();
    if !unsafe { xf::pixmith_xf_active(raw) } {
        return json!({ "active": false });
    }
    let mut f = [0f64; 7];
    unsafe { xf::pixmith_xf_fields(raw, f.as_mut_ptr()) };
    json!({ "active": true, "x": f[0], "y": f[1], "width": f[2], "height": f[3], "angle": f[4], "skew_x": f[5], "skew_y": f[6] })
}

fn free_transform(e: &mut Engine, a: XfArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let active = unsafe { xf::pixmith_xf_active(raw) };
    match a.action {
        XfAction::Begin => {
            let l = d.image_layer(&a.layer)?;
            let f = d.frame(a.frame)?;
            d.edit("Free Transform", Undo::Live, |_| {
                unsafe { xf::pixmith_xf_begin(raw, l, f, a.rotsprite.unwrap_or(true)) }
                    .then_some(())
                    .ok_or_else(|| fail("could not start a free transform (empty cel or selection?)"))
            })?;
            d.doc.label("Free Transform");
        }
        XfAction::Set => {
            if !active {
                return Err("no free transform is running; call action begin first".into());
            }
            d.edit("Free Transform", Undo::Live, |_| {
                unsafe {
                    if let Some(rs) = a.rotsprite {
                        xf::pixmith_xf_set_rotsprite(raw, rs);
                    }
                    let (hx, hy) = a.anchor.unwrap_or(Anchor::Center).dir();
                    xf::pixmith_xf_set_anchor(raw, hx, hy);
                    for (i, v) in [a.x, a.y, a.width, a.height, a.angle, a.skew_x, a.skew_y].into_iter().enumerate() {
                        if let Some(v) = v {
                            xf::pixmith_xf_set_field(raw, i as u32, v);
                        }
                    }
                }
                Ok(())
            })?;
        }
        XfAction::Info => {}
        XfAction::Commit => {
            if !active {
                return Err("no free transform is running".into());
            }
            d.edit("Free Transform", Undo::Live, |_| {
                unsafe { xf::pixmith_xf_commit(raw) };
                Ok(())
            })?;
        }
        XfAction::Cancel => {
            d.edit("Free Transform", Undo::Live, |_| {
                unsafe { xf::pixmith_xf_cancel(raw) }
                    .then_some(())
                    .ok_or_else(|| "no free transform is running".to_string())
            })?;
        }
    }
    let mut v = xf_state(d);
    v["doc"] = json!(d.id);
    Out::json(v)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SizeAction {
    Scale,
    Canvas,
}

#[derive(Deserialize, JsonSchema)]
pub struct SizeArgs {
    pub action: SizeAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// New width in pixels.
    pub width: Option<u32>,
    /// New height in pixels.
    pub height: Option<u32>,
    /// scale: multiply both sides by this (e.g. 2 or 0.5) instead of width/height.
    pub factor: Option<f64>,
    /// canvas: where the old image sits in the new canvas (default center).
    pub anchor: Option<Anchor>,
    /// canvas: explicit x of the old top-left in the new canvas (overrides anchor).
    pub offset_x: Option<i32>,
    /// canvas: explicit y of the old top-left.
    pub offset_y: Option<i32>,
}

fn sprite_size(e: &mut Engine, a: SizeArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let (w0, h0) = (d.doc.width(), d.doc.height());
    let (w, h) = match (a.factor, a.width, a.height) {
        (Some(f), _, _) if f > 0.0 => {
            (((w0 as f64 * f).round() as u32).max(1), ((h0 as f64 * f).round() as u32).max(1))
        }
        (_, Some(w), Some(h)) => (w, h),
        (_, Some(w), None) => {
            (w, if a.action == SizeAction::Scale { (h0 as u64 * w as u64 / w0 as u64).max(1) as u32 } else { h0 })
        }
        (_, None, Some(h)) => {
            (if a.action == SizeAction::Scale { (w0 as u64 * h as u64 / h0 as u64).max(1) as u32 } else { w0 }, h)
        }
        _ => return Err("give width and/or height (or factor)".into()),
    };
    if w == 0 || h == 0 || w > 16384 || h > 16384 {
        return Err("the new size must be 1-16384 on each side".into());
    }
    let raw = d.doc.raw();
    match a.action {
        SizeAction::Scale => d.edit("Scale Sprite", Undo::Step, |_| {
            unsafe { filters::pixmith_scale_sprite(raw, w, h) }.then_some(()).ok_or_else(|| fail("could not scale"))
        })?,
        SizeAction::Canvas => {
            let (ax, ay) = a.anchor.unwrap_or(Anchor::Center).dir();
            let place = |new: u32, old: u32, dir: i32| ((new as i64 - old as i64) * (dir as i64 + 1) / 2) as i32;
            let ox = a.offset_x.unwrap_or_else(|| place(w, w0, ax));
            let oy = a.offset_y.unwrap_or_else(|| place(h, h0, ay));
            d.edit("Canvas Size", Undo::Step, |_| {
                unsafe { sel::pixmith_resize_canvas(raw, w, h, ox, oy) }
                    .then_some(())
                    .ok_or_else(|| fail("could not resize the canvas"))
            })?
        }
    }
    Out::json(json!({ "doc": d.id, "width": d.doc.width(), "height": d.doc.height() }))
}
