//! Slices (named rectangles with optional 9-patch center and pivot) and user data.

use super::view::Rect;
use super::{CHANGE, Out, ToolDef, need, tool};
use crate::color::Color;
use crate::doc::{cstr, fail, take_string};
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use pixmith_core::document::slices as sf;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Slices & user data",
            "slice",
            "Slices",
            CHANGE,
            SliceArgs,
            slice,
            "Slices: named rectangles exported with sprite sheets and used for UI 9-patches and pivots. Actions: list, add (name, bounds), set (any of name, bounds, center = the 9-patch middle rectangle relative to the slice, pivot = [x, y] relative to the slice; clear_center / clear_pivot remove them), remove. Bounds, center and pivot are keyed per frame: a value set at `frame` applies from there until the next key (default frame 0)."
        ),
        tool!(
            "Slices & user data",
            "user_data",
            "User data",
            CHANGE,
            UdArgs,
            user_data,
            "Read or set the user data (text and color, as in Aseprite) of the sprite, a layer, a cel, a tag or a slice. Writing to an empty cel creates it."
        ),
    ]
}

pub fn slices_json(d: &DocEntry, frame: u32) -> Vec<Value> {
    let raw = d.doc.raw();
    let n = unsafe { sf::pixmith_slice_count(raw) };
    (0..n)
        .map(|i| {
            let name = unsafe { take_string(sf::pixmith_slice_name(raw, i)) };
            let mut o = [0i32; 13];
            let mut f = frame;
            let mut has = unsafe { sf::pixmith_slice_get(raw, i, f, o.as_mut_ptr()) };
            if !has {
                // No key at or before this frame: show the first key instead.
                for k in 0..d.doc.frame_count() {
                    if unsafe { sf::pixmith_slice_get(raw, i, k, o.as_mut_ptr()) } {
                        f = k;
                        has = true;
                        break;
                    }
                }
            }
            let mut v = json!({ "index": i, "name": name });
            if has {
                v["frame"] = json!(f);
                v["key_frame"] = json!(o[12]);
                v["bounds"] = json!({ "x": o[0], "y": o[1], "w": o[2], "h": o[3] });
                v["center"] =
                    if o[4] != 0 { json!({ "x": o[5], "y": o[6], "w": o[7], "h": o[8] }) } else { Value::Null };
                v["pivot"] = if o[9] != 0 { json!([o[10], o[11]]) } else { Value::Null };
            }
            let ud = super::docs::user_data(d, 4, i, 0);
            if !ud.is_null() {
                v["user_data"] = ud;
            }
            v
        })
        .collect()
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SliceAction {
    List,
    Add,
    Set,
    Remove,
}

#[derive(Deserialize, JsonSchema)]
pub struct SliceArgs {
    pub action: SliceAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// set/remove: slice index or name.
    pub slice: Option<Ref>,
    /// add/set: name.
    pub name: Option<String>,
    /// add/set: rectangle on the canvas.
    pub bounds: Option<Rect>,
    /// set: 9-patch center rectangle, relative to the slice's top-left.
    pub center: Option<Rect>,
    /// set: pivot [x, y] relative to the slice's top-left.
    pub pivot: Option<[i32; 2]>,
    /// set: remove the 9-patch center.
    pub clear_center: Option<bool>,
    /// set: remove the pivot.
    pub clear_pivot: Option<bool>,
    /// Frame the bounds/center/pivot key applies from (default 0).
    pub frame: Option<u32>,
}

fn slice(e: &mut Engine, a: SliceArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let frame = a.frame.unwrap_or(0);
    d.frame(Some(frame))?;
    match a.action {
        SliceAction::List => {}
        SliceAction::Add => {
            let b = need(a.bounds, "bounds", &act)?;
            let name = a.name.clone();
            d.edit("New Slice", Undo::Step, |_| {
                let i = unsafe { sf::pixmith_slice_add(raw, b.x, b.y, b.w.max(1), b.h.max(1)) };
                if i < 0 {
                    return Err(fail("could not add the slice"));
                }
                if let Some(n) = &name {
                    let c = cstr(n);
                    unsafe {
                        sf::pixmith_slice_set_props(raw, i as u32, 0, c.as_ptr(), std::ptr::null(), std::ptr::null())
                    };
                }
                Ok(())
            })?;
        }
        SliceAction::Set => {
            let i = d.slice(&need(a.slice.clone(), "slice", &act)?)?;
            let mut o = [0i32; 13];
            unsafe { sf::pixmith_slice_get(raw, i, frame, o.as_mut_ptr()) };
            let name = a.name.clone().unwrap_or_else(|| unsafe { take_string(sf::pixmith_slice_name(raw, i)) });
            let center: Option<[i32; 4]> = match (a.center, a.clear_center.unwrap_or(false)) {
                (_, true) => None,
                (Some(c), _) => Some([c.x, c.y, c.w as i32, c.h as i32]),
                (None, _) => (o[4] != 0).then_some([o[5], o[6], o[7], o[8]]),
            };
            let pivot: Option<[i32; 2]> = match (a.pivot, a.clear_pivot.unwrap_or(false)) {
                (_, true) => None,
                (Some(p), _) => Some(p),
                (None, _) => (o[9] != 0).then_some([o[10], o[11]]),
            };
            let c = cstr(&name);
            d.edit("Slice Properties", Undo::Step, |_| {
                if let Some(b) = a.bounds
                    && !unsafe { sf::pixmith_slice_set_bounds(raw, i, frame, b.x, b.y, b.w.max(1), b.h.max(1)) }
                {
                    return Err(fail("could not set the bounds"));
                }
                let cp = center.as_ref().map_or(std::ptr::null(), |c| c.as_ptr());
                let pp = pivot.as_ref().map_or(std::ptr::null(), |p| p.as_ptr());
                unsafe { sf::pixmith_slice_set_props(raw, i, frame, c.as_ptr(), cp, pp) }
                    .then_some(())
                    .ok_or_else(|| fail("could not set the slice properties"))
            })?;
        }
        SliceAction::Remove => {
            let i = d.slice(&need(a.slice.clone(), "slice", &act)?)?;
            d.edit("Delete Slice", Undo::Step, |_| {
                unsafe { sf::pixmith_slice_remove(raw, i) }
                    .then_some(())
                    .ok_or_else(|| fail("could not remove the slice"))
            })?;
        }
    }
    Out::json(json!({ "doc": d.id, "slices": slices_json(d, frame) }))
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UdTarget {
    Sprite,
    Layer,
    Cel,
    Tag,
    Slice,
}

#[derive(Deserialize, JsonSchema)]
pub struct UdArgs {
    /// get (default when nothing is given to set) or set.
    pub action: Option<UdAction>,
    pub target: UdTarget,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// layer/cel target: layer (default: active).
    pub layer: Option<Ref>,
    /// cel target: frame (default: active).
    pub frame: Option<u32>,
    /// tag target: tag index or name.
    pub tag: Option<Ref>,
    /// slice target: slice index or name.
    pub slice: Option<Ref>,
    /// set: text ("" clears; omitted keeps the current text).
    pub text: Option<String>,
    /// set: color (omitted keeps it).
    pub color: Option<Color>,
    /// set: remove the color.
    pub clear_color: Option<bool>,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum UdAction {
    Get,
    Set,
}

fn user_data(e: &mut Engine, a: UdArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let (kind, x, y) = match a.target {
        UdTarget::Sprite => (0, 0, 0),
        UdTarget::Layer => (1, d.layer(&a.layer)?, 0),
        UdTarget::Cel => (2, d.frame(a.frame)?, d.image_layer(&a.layer)?),
        UdTarget::Tag => (3, d.tag(a.tag.as_ref().ok_or("target tag needs `tag`")?)?, 0),
        UdTarget::Slice => (4, d.slice(a.slice.as_ref().ok_or("target slice needs `slice`")?)?, 0),
    };
    let set = a.action == Some(UdAction::Set)
        || (a.action.is_none() && (a.text.is_some() || a.color.is_some() || a.clear_color.is_some()));
    if set {
        let full = unsafe { take_string(sf::pixmith_userdata_text(raw, kind, x, y)) };
        // Keep Pixmith's own lines (terrains, tweens, export settings) when replacing the text.
        let own: Vec<&str> = full.lines().filter(|l| l.starts_with("pixmith.") || l.starts_with("pixmith-")).collect();
        let user: Vec<&str> =
            full.lines().filter(|l| !l.starts_with("pixmith.") && !l.starts_with("pixmith-")).collect();
        let new_user = a.text.clone().unwrap_or_else(|| user.join("\n"));
        let text = if own.is_empty() {
            new_user
        } else if new_user.is_empty() {
            own.join("\n")
        } else {
            format!("{new_user}\n{}", own.join("\n"))
        };
        let mut old = 0u32;
        let had = unsafe { sf::pixmith_userdata_color(raw, kind, x, y, &mut old) };
        let (has_color, rgba) = match (a.color, a.clear_color.unwrap_or(false)) {
            (_, true) => (false, 0),
            (Some(c), _) => (true, c.0),
            (None, _) => (had, old),
        };
        let c = cstr(&text);
        d.edit("User Data", Undo::Step, |_| {
            unsafe { sf::pixmith_userdata_set(raw, kind, x, y, c.as_ptr(), has_color, rgba) }
                .then_some(())
                .ok_or_else(|| fail("could not set the user data"))
        })?;
    }
    Out::json(
        json!({ "doc": d.id, "target": super::act(&a.target), "user_data": super::docs::user_data(d, kind, x, y) }),
    )
}
