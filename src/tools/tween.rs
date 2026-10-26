//! Tweening: keyframed position, rotation, scale and opacity per layer, eased, then baked into frames.

use super::{CHANGE, Out, ToolDef, need, tool};
use crate::doc::{fail, take_string};
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use pixmith_core::gamedev::tween as tw;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Tweening",
        "tween",
        "Tween keys and baking",
        CHANGE,
        TweenArgs,
        tween,
        "Animate a layer (or group) by keyframes instead of redrawing it: properties x, y (offset in pixels), rotation (degrees, clockwise), scale_x, scale_y (1 = 100%), opacity (0-1). Between keys the value follows the key's ease: linear, hold, in_quad, out_quad, in_out_quad, in_out_cubic, out_back, out_bounce, or bezier with control points [x1, y1, x2, y2]. Actions: info (keys, pivot, options, the transform at `frame`), set_key, set_keys (several properties at one frame), remove_key (alias: delete_key), move_key, set_ease, set_pivot / reset_pivot (rotation/scale center), set_options (rotsprite, snap_scale, ignore_opacity), copy_keys / paste_keys, bake (render the tween into real cels: mode all / range / frame; new_layer puts them on a new layer; keep_source keeps the keys), bake_all. Tweened layers already render and export animated; bake when you need real per-frame pixels to edit."
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Prop {
    X,
    Y,
    Rotation,
    ScaleX,
    ScaleY,
    Opacity,
}

const PROPS: [&str; 6] = ["x", "y", "rotation", "scale_x", "scale_y", "opacity"];
const EASES: [&str; 9] =
    ["linear", "hold", "in_quad", "out_quad", "in_out_quad", "in_out_cubic", "out_back", "out_bounce", "bezier"];

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Ease {
    Linear,
    Hold,
    InQuad,
    OutQuad,
    InOutQuad,
    InOutCubic,
    OutBack,
    OutBounce,
    Bezier,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TweenAction {
    Info,
    SetKey,
    SetKeys,
    #[serde(alias = "delete_key")]
    RemoveKey,
    MoveKey,
    SetEase,
    SetPivot,
    ResetPivot,
    SetOptions,
    CopyKeys,
    PasteKeys,
    Bake,
    BakeAll,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BakeMode {
    All,
    Range,
    Frame,
}

#[derive(Deserialize, JsonSchema, Default)]
pub struct Values {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub rotation: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub opacity: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TweenArgs {
    pub action: TweenAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer or group (default: active).
    pub layer: Option<Ref>,
    /// Key frame (default: active); info: frame to evaluate; paste_keys: where to paste; bake mode frame: the frame.
    pub frame: Option<u32>,
    /// set_key/remove_key/move_key/set_ease: the property.
    pub property: Option<Prop>,
    /// set_key: value.
    pub value: Option<f32>,
    /// set_keys: values by property.
    pub values: Option<Values>,
    /// Ease from this key to the next (default linear).
    pub ease: Option<Ease>,
    /// bezier ease: [x1, y1, x2, y2] (like CSS cubic-bezier).
    pub bezier: Option<[f32; 4]>,
    /// move_key/copy_keys/bake range: first frame.
    pub from: Option<u32>,
    /// move_key: destination; copy_keys/bake range: last frame.
    pub to: Option<u32>,
    /// set_pivot: pivot in canvas pixels.
    pub x: Option<f32>,
    /// set_pivot: pivot in canvas pixels.
    pub y: Option<f32>,
    /// set_options: RotSprite rotation.
    pub rotsprite: Option<bool>,
    /// set_options: round scale to whole multiples.
    pub snap_scale: Option<bool>,
    /// set_options: ignore the opacity track.
    pub ignore_opacity: Option<bool>,
    /// bake: all (default), range (from..to) or frame.
    pub mode: Option<BakeMode>,
    /// bake: put the baked cels on a new layer (groups always do).
    pub new_layer: Option<bool>,
    /// bake: keep the keys and the source cel.
    pub keep_source: Option<bool>,
}

fn info(d: &DocEntry, l: u32, frame: u32) -> Value {
    let raw = d.doc.raw();
    let mut tracks = serde_json::Map::new();
    for (p, name) in PROPS.iter().enumerate() {
        let n = unsafe { tw::pixmith_tween_key_count(raw, l, p as u32) };
        let mut keys = Vec::new();
        for i in 0..n {
            let mut k = [0f32; 7];
            if unsafe { tw::pixmith_tween_key_at(raw, l, p as u32, i, k.as_mut_ptr()) } {
                let ease = EASES.get(k[2] as usize).copied().unwrap_or("linear");
                let mut v = json!({ "frame": k[0] as u32, "value": k[1], "ease": ease });
                if ease == "bezier" {
                    v["bezier"] = json!([k[3], k[4], k[5], k[6]]);
                }
                keys.push(v);
            }
        }
        if !keys.is_empty() {
            tracks.insert(name.to_string(), Value::Array(keys));
        }
    }
    let o = unsafe { tw::pixmith_tween_options(raw, l) };
    let mut piv = [0f32; 2];
    let has_pivot = unsafe { tw::pixmith_tween_get_pivot(raw, l, piv.as_mut_ptr()) };
    let mut t = [0f32; 8];
    let has_t = unsafe { tw::pixmith_tween_transform_at(raw, l, frame, t.as_mut_ptr()) };
    let mut range = [0u32; 2];
    let has_range = unsafe { tw::pixmith_tween_range(raw, l, range.as_mut_ptr()) };
    let warning = unsafe { take_string(tw::pixmith_tween_warning_text(raw, l)) };
    json!({
        "layer": l,
        "tracks": tracks,
        "key_range": has_range.then_some(range),
        "pivot": has_pivot.then_some(piv),
        "custom_pivot": o & 4 != 0,
        "options": { "rotsprite": o & 2 != 0, "snap_scale": o & 8 != 0, "ignore_opacity": o & 16 != 0 },
        "at_frame": has_t.then(|| json!({ "frame": frame, "x": t[0], "y": t[1], "rotation": t[2], "scale_x": t[3], "scale_y": t[4], "opacity": t[5] })),
        "warning": (!warning.is_empty()).then_some(warning),
    })
}

fn tween(e: &mut Engine, a: TweenArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let l = d.layer(&a.layer)?;
    let f = d.frame(a.frame)?;
    let ease = a.ease.unwrap_or(Ease::Linear);
    let bz = a.bezier.unwrap_or([0.25, 0.1, 0.25, 1.0]);
    if ease == Ease::Bezier && a.bezier.is_none() {
        return Err("ease bezier needs `bezier` [x1, y1, x2, y2]".into());
    }
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(fail(what.to_string())) };
    let mut extra = Value::Null;
    match a.action {
        TweenAction::Info => {}
        TweenAction::SetKey => {
            let p = need(a.property, "property", &act)? as u32;
            let v = need(a.value, "value", &act)?;
            d.edit("Set Tween Key", Undo::Step, |_| {
                ok(
                    unsafe { tw::pixmith_tween_set_key(raw, l, p, f, v, ease as u32, bz[0], bz[1], bz[2], bz[3]) },
                    "could not set the key",
                )
            })?;
        }
        TweenAction::SetKeys => {
            let vals = need(a.values.as_ref(), "values", &act)?;
            let list = [vals.x, vals.y, vals.rotation, vals.scale_x, vals.scale_y, vals.opacity];
            let mask = list.iter().enumerate().filter(|(_, v)| v.is_some()).fold(0u32, |m, (i, _)| m | 1 << i);
            if mask == 0 {
                return Err("`values` sets no property".into());
            }
            let defaults = [0.0, 0.0, 0.0, 1.0, 1.0, 1.0];
            let arr: Vec<f32> = list.iter().zip(defaults).map(|(v, dflt)| v.unwrap_or(dflt)).collect();
            if ease == Ease::Bezier {
                // set_keys_at takes no Bezier handles: set the keys one by one.
                d.edit("Set Tween Keys", Undo::Step, |_| {
                    for (p, v) in list.iter().enumerate() {
                        if let Some(v) = v {
                            ok(
                                unsafe {
                                    tw::pixmith_tween_set_key(raw, l, p as u32, f, *v, 8, bz[0], bz[1], bz[2], bz[3])
                                },
                                "could not set a key",
                            )?;
                        }
                    }
                    Ok(())
                })?;
            } else {
                d.edit("Set Tween Keys", Undo::Step, |_| {
                    ok(
                        unsafe { tw::pixmith_tween_set_keys_at(raw, l, f, mask, arr.as_ptr(), ease as u32) },
                        "could not set the keys",
                    )
                })?;
            }
        }
        TweenAction::RemoveKey => {
            let p = need(a.property, "property", &act)? as u32;
            d.edit("Remove Tween Key", Undo::Step, |_| {
                ok(unsafe { tw::pixmith_tween_remove_key(raw, l, p, f) }, "no key at that frame")
            })?;
        }
        TweenAction::MoveKey => {
            let p = need(a.property, "property", &act)? as u32;
            let from = need(a.from, "from", &act)?;
            let to = d.frame(Some(need(a.to, "to", &act)?))?;
            d.edit("Move Tween Key", Undo::Step, |_| {
                ok(unsafe { tw::pixmith_tween_move_key(raw, l, p, from, to) }, "could not move the key")
            })?;
        }
        TweenAction::SetEase => {
            let p = need(a.property, "property", &act)? as u32;
            d.edit("Change Tween Curve", Undo::Step, |_| {
                ok(
                    unsafe { tw::pixmith_tween_set_ease(raw, l, p, f, ease as u32, bz[0], bz[1], bz[2], bz[3]) },
                    "no key at that frame",
                )
            })?;
        }
        TweenAction::SetPivot => {
            let (x, y) = (need(a.x, "x", &act)?, need(a.y, "y", &act)?);
            d.edit("Set Tween Pivot", Undo::Step, |_| {
                ok(unsafe { tw::pixmith_tween_set_pivot(raw, l, x, y, true) }, "could not set the pivot")
            })?;
        }
        TweenAction::ResetPivot => {
            d.edit("Reset Tween Pivot", Undo::Step, |_| {
                ok(unsafe { tw::pixmith_tween_set_pivot(raw, l, 0.0, 0.0, false) }, "could not reset the pivot")
            })?;
        }
        TweenAction::SetOptions => {
            d.edit("Tween Option", Undo::Step, |_| {
                for (flag, v) in [(0, a.rotsprite), (1, a.snap_scale), (2, a.ignore_opacity)] {
                    if let Some(v) = v {
                        ok(unsafe { tw::pixmith_tween_set_flag(raw, l, flag, v) }, "could not set the option")?;
                    }
                }
                Ok(())
            })?;
        }
        TweenAction::CopyKeys => {
            let from = a.from.unwrap_or(0);
            let to = a.to.unwrap_or(d.doc.frame_count().saturating_sub(1));
            let n = unsafe { tw::pixmith_tween_copy_keys(raw, l, from, to) };
            extra = json!({ "copied_keys": n });
        }
        TweenAction::PasteKeys => {
            let n = d.edit("Paste Tween Keys", Undo::Step, |_| {
                let n = unsafe { tw::pixmith_tween_paste_keys(raw, l, f) };
                if n == 0 { Err("nothing to paste; copy_keys first".to_string()) } else { Ok(n) }
            })?;
            extra = json!({ "pasted_keys": n });
        }
        TweenAction::Bake => {
            let mode = a.mode.unwrap_or(BakeMode::All);
            let (m, from, to) = match mode {
                BakeMode::All => (0, 0, 0),
                BakeMode::Range => (1, need(a.from, "from", &act)?, need(a.to, "to", &act)?),
                BakeMode::Frame => (2, f, f),
            };
            let keep = a.keep_source.unwrap_or(false);
            let nl = a.new_layer.unwrap_or(false);
            let out = d.edit(if keep { "Bake Tween (keep source)" } else { "Bake Tween" }, Undo::Step, |_| {
                let r = unsafe { tw::pixmith_tween_bake_ex(raw, l, m, from, to, nl, keep) };
                if r < 0 { Err(fail("could not bake (does the layer have keys?)")) } else { Ok(r as u32) }
            })?;
            extra = json!({ "baked_into_layer": out });
        }
        TweenAction::BakeAll => {
            let keep = a.keep_source.unwrap_or(false);
            let nl = a.new_layer.unwrap_or(false);
            let n = d.edit("Bake All Tweens", Undo::Step, |_| {
                let n = unsafe { tw::pixmith_tween_bake_all(raw, keep, nl) };
                if n == 0 { Err("no layer has tween keys".to_string()) } else { Ok(n) }
            })?;
            extra = json!({ "layers_baked": n });
        }
    }
    let mut v = info(d, l, f);
    v["doc"] = json!(d.id);
    if !extra.is_null() {
        v["result"] = extra;
    }
    Out::json(v)
}
