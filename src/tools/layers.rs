//! Layers and groups, frames, cels and animation tags.

use super::docs::{BLEND_NAMES, tags_json};
use super::{CHANGE, Out, ToolDef, need, tool};
use crate::color::Color;
use crate::doc::{cstr, fail};
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use pixmith_core::document::layers as lf;
use pixmith_core::ffi::document as core;
use pixmith_core::formats::import;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Layers & frames",
            "layer",
            "Layers and groups",
            CHANGE,
            LayerArgs,
            layer,
            "Edit the layer stack (index 0 = bottom; a group comes right below its children in the list). Actions: add (new image layer above `layer`, default above the active one), add_group, duplicate, remove (a group with its children), move_up / move_down (one step), move_to (to stack position `to`), into_group (move into `group`), out_of_group, wrap_in_group (put the layer in a new group), merge_down, set (any of name, visible, opacity 0-255, blend_mode), import_png (PNG file as a new layer). New layers become the active layer. Returns the updated layer list."
        ),
        tool!(
            "Layers & frames",
            "frame",
            "Frames",
            CHANGE,
            FrameArgs,
            frame,
            "Edit animation frames (0-based). Actions: add (`count` empty frames after `frame`, default after the active one), duplicate (copy `frame` right after itself, `count` times), remove, move (frame `frame` to position `to`), set_duration (milliseconds for `frame`, `frames` list, or every frame with all=true). Tags are adjusted automatically."
        ),
        tool!(
            "Layers & frames",
            "cel",
            "Cels",
            CHANGE,
            CelArgs,
            cel,
            "Edit cels (the image of one layer in one frame). Actions: link (frames from_frame..to_frame of `layer` share the image of from_frame), unlink, set_opacity, clear (remove the images of a block of layers x frames), copy (a block to the cel clipboard), paste (clipboard at layer/frame), move (shift a block by dl layers / df frames; copy=true duplicates)."
        ),
        tool!(
            "Layers & frames",
            "tag",
            "Animation tags",
            CHANGE,
            TagArgs,
            tag,
            "Animation tags (named frame ranges used for playback and exports). Actions: add (name, from, to), update (any of name, from, to, direction, repeat, color), remove. direction: forward, reverse, ping_pong, ping_pong_reverse; repeat 0 = forever."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum LayerAction {
    Add,
    AddGroup,
    Duplicate,
    Remove,
    MoveUp,
    MoveDown,
    MoveTo,
    IntoGroup,
    OutOfGroup,
    WrapInGroup,
    MergeDown,
    Set,
    ImportPng,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    Hue,
    Saturation,
    Color,
    Luminosity,
    Addition,
    Subtract,
    Divide,
}

#[derive(Deserialize, JsonSchema)]
pub struct LayerArgs {
    pub action: LayerAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// The layer to act on, index or name (default: active). For add/add_group/import_png: the new layer goes above it.
    pub layer: Option<Ref>,
    /// add/add_group/set: layer name.
    pub name: Option<String>,
    /// set: visibility.
    pub visible: Option<bool>,
    /// set: opacity 0-255.
    pub opacity: Option<u8>,
    /// set: blend mode.
    pub blend_mode: Option<BlendMode>,
    /// move_to: target stack position.
    pub to: Option<u32>,
    /// into_group: the group (index or name).
    pub group: Option<Ref>,
    /// import_png: PNG file.
    pub path: Option<String>,
    /// import_png: frame to put it in (default: active).
    pub frame: Option<u32>,
}

/// Compact layer list for results.
pub fn layer_list(d: &DocEntry) -> Vec<Value> {
    super::docs::layers_json(d)
        .into_iter()
        .map(|l| json!({ "index": l["index"], "name": l["name"], "kind": l["kind"], "level": l["level"], "visible": l["visible"] }))
        .collect()
}

fn layer(e: &mut Engine, a: LayerArgs) -> R<Out> {
    let src = match (a.action, &a.path) {
        (LayerAction::ImportPng, Some(p)) => Some(e.sandbox.read(p)?),
        (LayerAction::ImportPng, None) => return Err("import_png needs `path`".into()),
        _ => None,
    };
    let d = e.doc(&a.doc)?;
    let act = super::act(&a.action);
    let raw = d.doc.raw();
    let l = d.layer(&a.layer)?;
    let result = |r: i32, what: &str| -> R<u32> {
        if r < 0 { Err(fail(format!("cannot {what} layer {l}"))) } else { Ok(r as u32) }
    };
    let rename = |i: u32, name: &Option<String>| {
        if let Some(n) = name {
            let c = cstr(n);
            unsafe { core::pixmith_rename_layer(raw, i, c.as_ptr()) };
        }
    };
    let new_index: Option<u32> = match a.action {
        LayerAction::Add => Some(d.edit("New Layer", Undo::Step, |_| {
            let i = unsafe { core::pixmith_add_layer(raw, l) };
            rename(i, &a.name);
            Ok(i)
        })?),
        LayerAction::AddGroup => Some(d.edit("New Group", Undo::Step, |_| {
            let i = result(unsafe { lf::pixmith_layer_add_group(raw, l) }, "add a group above")?;
            rename(i, &a.name);
            Ok(i)
        })?),
        LayerAction::Duplicate => Some(d.edit("Duplicate Layer", Undo::Step, |_| {
            result(unsafe { lf::pixmith_layer_duplicate(raw, l) }, "duplicate")
        })?),
        LayerAction::Remove => {
            d.edit("Delete Layer", Undo::Step, |_| {
                unsafe { core::pixmith_remove_layer(raw, l) }
                    .then_some(())
                    .ok_or_else(|| fail(format!("cannot remove layer {l} (the last image layer cannot be removed)")))
            })?;
            d.clamp_active();
            None
        }
        LayerAction::MoveUp | LayerAction::MoveDown => {
            let up = a.action == LayerAction::MoveUp;
            Some(d.edit("Move Layer", Undo::Step, |_| result(unsafe { lf::pixmith_layer_move(raw, l, up) }, "move"))?)
        }
        LayerAction::MoveTo => {
            let to = need(a.to, "to", &act)?;
            Some(d.edit("Move Layer", Undo::Step, |_| {
                let mut cur = l;
                let mut guard = 0;
                while cur != to && guard < 4096 {
                    guard += 1;
                    let r = unsafe { lf::pixmith_layer_move(raw, cur, to > cur) };
                    if r < 0 || r as u32 == cur {
                        break;
                    }
                    cur = r as u32;
                }
                Ok(cur)
            })?)
        }
        LayerAction::IntoGroup => {
            let g = d.layer(&Some(need(a.group.clone(), "group", &act)?))?;
            if !d.is_group(g) {
                return Err(format!("layer {g} is not a group"));
            }
            Some(d.edit("Move into Group", Undo::Step, |_| {
                result(unsafe { lf::pixmith_layer_into_group(raw, l, g) }, "move into the group")
            })?)
        }
        LayerAction::OutOfGroup => Some(d.edit("Move out of Group", Undo::Step, |_| {
            result(unsafe { lf::pixmith_layer_out_of_group(raw, l) }, "move out of its group")
        })?),
        LayerAction::WrapInGroup => Some(d.edit("Group Layer", Undo::Step, |_| {
            let i = result(unsafe { lf::pixmith_layer_group(raw, l) }, "group")?;
            Ok(i)
        })?),
        LayerAction::MergeDown => {
            if !unsafe { lf::pixmith_layer_can_merge_down(raw, l) } {
                return Err(format!(
                    "layer {l} cannot be merged down (needs an image layer directly below it in the same group)"
                ));
            }
            Some(d.edit("Merge Down", Undo::Step, |_| {
                result(unsafe { lf::pixmith_layer_merge_down(raw, l) }, "merge down")
            })?)
        }
        LayerAction::Set => {
            let name = a.name.clone().unwrap_or_else(|| d.layer_name(l));
            let opacity = a.opacity.unwrap_or(unsafe { core::pixmith_layer_opacity(raw, l) });
            let blend = a.blend_mode.map_or(unsafe { lf::pixmith_layer_blend_mode(raw, l) }, |b| b as u16);
            let visible = a.visible.unwrap_or(unsafe { core::pixmith_layer_visible(raw, l) });
            let c = cstr(&name);
            d.edit("Layer Properties", Undo::Step, |_| {
                unsafe { lf::pixmith_layer_set_props(raw, l, c.as_ptr(), opacity, blend, visible) }
                    .then_some(())
                    .ok_or_else(|| fail("could not set the layer properties"))
            })?;
            None
        }
        LayerAction::ImportPng => {
            let f = d.frame(a.frame)?;
            let c = cstr(&src.expect("checked").to_string_lossy());
            Some(d.edit("Import PNG as Layer", Undo::Step, |_| {
                let i = unsafe { import::pixmith_import_png_layer(raw, l, f, c.as_ptr()) };
                if i < 0 {
                    return Err(fail("could not import the PNG"));
                }
                rename(i as u32, &a.name);
                Ok(i as u32)
            })?)
        }
    };
    if let Some(i) = new_index {
        d.layer = i;
    }
    d.clamp_active();
    let mut v = json!({ "doc": d.id, "active_layer": d.layer, "layers": layer_list(d) });
    if let Some(i) = new_index {
        v["layer"] = json!(i);
    }
    if a.action == LayerAction::Set {
        v["layer"] = json!(l);
        v["blend_mode"] = json!(BLEND_NAMES[unsafe { lf::pixmith_layer_blend_mode(raw, l) } as usize % 19]);
        v["opacity"] = json!(unsafe { core::pixmith_layer_opacity(raw, l) });
    }
    Out::json(v)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum FrameAction {
    Add,
    Duplicate,
    Remove,
    Move,
    SetDuration,
}

#[derive(Deserialize, JsonSchema)]
pub struct FrameArgs {
    pub action: FrameAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// The frame to act on (default: active). add inserts after it.
    pub frame: Option<u32>,
    /// add/duplicate: how many (default 1, max 1024).
    pub count: Option<u32>,
    /// move: destination index.
    pub to: Option<u32>,
    /// set_duration: milliseconds (1-65535).
    pub duration_ms: Option<u32>,
    /// set_duration: these frames instead of `frame`.
    pub frames: Option<Vec<u32>>,
    /// set_duration: every frame.
    pub all: Option<bool>,
}

fn frame(e: &mut Engine, a: FrameArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let f = d.frame(a.frame)?;
    match a.action {
        FrameAction::Add | FrameAction::Duplicate => {
            let n = a.count.unwrap_or(1).clamp(1, 1024);
            let dup = a.action == FrameAction::Duplicate;
            let last = d.edit(if dup { "Duplicate Frame" } else { "New Frame" }, Undo::Step, |_| {
                let mut at = f;
                for _ in 0..n {
                    // A duplicate copies the frame before it, so repeated copies all show the original.
                    at = unsafe { core::pixmith_add_frame(raw, at, dup) };
                }
                Ok(at)
            })?;
            d.frame = if dup { f + 1 } else { last + 1 - n };
        }
        FrameAction::Remove => {
            d.edit("Delete Frame", Undo::Step, |_| {
                unsafe { core::pixmith_remove_frame(raw, f) }
                    .then_some(())
                    .ok_or_else(|| fail("cannot remove the last frame"))
            })?;
            d.clamp_active();
        }
        FrameAction::Move => {
            let to = need(a.to, "to", &act)?;
            d.frame(Some(to))?;
            d.edit("Move Frame", Undo::Step, |_| {
                unsafe { lf::pixmith_frame_move(raw, f, to) }
                    .then_some(())
                    .ok_or_else(|| fail("could not move the frame"))
            })?;
            d.frame = to;
        }
        FrameAction::SetDuration => {
            let ms = need(a.duration_ms, "duration_ms", &act)?.clamp(1, 65535) as u16;
            let list: Vec<u32> = if a.all.unwrap_or(false) {
                (0..d.doc.frame_count()).collect()
            } else if let Some(fs) = &a.frames {
                fs.iter().map(|&x| d.frame(Some(x))).collect::<R<_>>()?
            } else {
                vec![f]
            };
            d.edit("Frame Duration", Undo::Step, |_| {
                for &x in &list {
                    unsafe { core::pixmith_set_frame_duration(raw, x, ms) };
                }
                Ok(())
            })?;
        }
    }
    let n = d.doc.frame_count();
    let durations: Vec<u16> = (0..n.min(512)).map(|i| unsafe { core::pixmith_frame_duration(raw, i) }).collect();
    Out::json(
        json!({ "doc": d.id, "frames": n, "active_frame": d.frame, "durations_ms": durations, "tags": tags_json(d) }),
    )
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum CelAction {
    Link,
    Unlink,
    SetOpacity,
    Clear,
    Copy,
    Paste,
    Move,
    Info,
}

#[derive(Deserialize, JsonSchema)]
pub struct CelArgs {
    pub action: CelAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer (default: active); the first layer of a block.
    pub layer: Option<Ref>,
    /// Frame (default: active); the first frame of a block.
    pub frame: Option<u32>,
    /// Block end layer for clear/copy/move (inclusive, default = layer).
    pub to_layer: Option<Ref>,
    /// link: first frame (default = frame). Block end for clear/copy/move is `to_frame`.
    pub from_frame: Option<u32>,
    /// link: last frame; clear/copy/move: block end frame (inclusive, default = frame).
    pub to_frame: Option<u32>,
    /// set_opacity: 0-255.
    pub opacity: Option<u8>,
    /// move: layers to shift by.
    pub dl: Option<i32>,
    /// move: frames to shift by.
    pub df: Option<i32>,
    /// move: copy instead of moving.
    pub copy: Option<bool>,
}

fn cel(e: &mut Engine, a: CelArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let l = d.layer(&a.layer)?;
    let f = d.frame(a.frame)?;
    let l1 = match &a.to_layer {
        Some(_) => d.layer(&a.to_layer)?,
        None => l,
    };
    let f1 = d.frame(Some(a.to_frame.unwrap_or(f)))?;
    let (l0, l1) = (l.min(l1), l.max(l1));
    let (f0, f1b) = (f.min(f1), f.max(f1));
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(fail(what.to_string())) };
    match a.action {
        CelAction::Link => {
            let from = d.frame(Some(a.from_frame.unwrap_or(f)))?;
            let to = d.frame(Some(need(a.to_frame, "to_frame", &act)?))?;
            d.image_layer(&Some(Ref::Index(l)))?;
            if to <= from {
                return Err("to_frame must be after from_frame".into());
            }
            if unsafe { lf::pixmith_cel_opacity(raw, l, from) } < 0 {
                return Err(format!("frame {from} has no image on layer {l} to link (see doc_info cels)"));
            }
            d.edit("Link Cels", Undo::Step, |_| {
                ok(unsafe { lf::pixmith_cel_link(raw, l, from, to) }, "could not link the cels")
            })?;
        }
        CelAction::Unlink => d.edit("Unlink Cel", Undo::Step, |_| {
            ok(unsafe { lf::pixmith_cel_unlink(raw, l, f) }, "the cel is not linked")
        })?,
        CelAction::SetOpacity => {
            let o = need(a.opacity, "opacity", &act)?;
            d.edit("Cel Opacity", Undo::Step, |_| {
                ok(unsafe { lf::pixmith_cel_set_opacity(raw, l, f, o) }, "the cel is empty")
            })?;
        }
        CelAction::Clear => d.edit("Clear Cels", Undo::Step, |_| {
            ok(unsafe { lf::pixmith_cels_clear(raw, l0, l1, f0, f1b) }, "nothing to clear")
        })?,
        CelAction::Copy => ok(unsafe { lf::pixmith_cels_copy(raw, l0, l1, f0, f1b) }, "nothing to copy")?,
        CelAction::Paste => {
            if !unsafe { lf::pixmith_cels_has_clip() } {
                return Err("the cel clipboard is empty; copy first".into());
            }
            d.edit("Paste Cels", Undo::Step, |_| {
                ok(unsafe { lf::pixmith_cels_paste(raw, l, f) }, "could not paste the cels")
            })?;
        }
        CelAction::Move => {
            let (dl, df) = (a.dl.unwrap_or(0), a.df.unwrap_or(0));
            let copy = a.copy.unwrap_or(false);
            d.edit(if copy { "Copy Cels" } else { "Move Cels" }, Undo::Step, |_| {
                ok(unsafe { lf::pixmith_cels_move(raw, l0, l1, f0, f1b, dl, df, copy) }, "could not move the cels")
            })?;
        }
        CelAction::Info => {}
    }
    let op = unsafe { lf::pixmith_cel_opacity(raw, l, f) };
    Out::json(json!({
        "doc": d.id, "layer": l, "frame": f,
        "exists": op >= 0, "opacity": (op >= 0).then_some(op),
        "linked": unsafe { lf::pixmith_cel_is_linked(raw, l, f) },
        "user_data": super::docs::user_data(d, 2, f, l),
    }))
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TagAction {
    Add,
    Update,
    Remove,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Forward,
    Reverse,
    PingPong,
    PingPongReverse,
}

#[derive(Deserialize, JsonSchema)]
pub struct TagArgs {
    pub action: TagAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// update/remove: the tag (index or name).
    pub tag: Option<Ref>,
    /// add/update: tag name.
    pub name: Option<String>,
    /// First frame (0-based, inclusive).
    pub from: Option<u32>,
    /// Last frame (inclusive).
    pub to: Option<u32>,
    pub direction: Option<Direction>,
    /// Times to play, 0 = forever.
    pub repeat: Option<u16>,
    /// Tag color in the timeline (alpha ignored).
    pub color: Option<Color>,
}

fn tag(e: &mut Engine, a: TagArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let act = super::act(&a.action);
    let n = d.doc.frame_count();
    let check = |f: u32| if f < n { Ok(f) } else { Err(format!("frame {f} does not exist ({n} frames)")) };
    match a.action {
        TagAction::Add => {
            let name = need(a.name.clone(), "name", &act)?;
            let from = check(need(a.from, "from", &act)?)?;
            let to = check(a.to.unwrap_or(from))?;
            let info = lf::PixmithTag {
                from: from.min(to),
                to: from.max(to),
                direction: a.direction.map_or(0, |x| x as u8),
                repeat: a.repeat.unwrap_or(0),
                color: a.color.map_or(0x7F7F7F, |c| c.0 >> 8),
            };
            let c = cstr(&name);
            d.edit("New Tag", Undo::Step, |_| {
                let r = unsafe { lf::pixmith_tag_add(raw, c.as_ptr(), info) };
                if r < 0 { Err(fail("could not add the tag")) } else { Ok(()) }
            })?;
        }
        TagAction::Update => {
            let i = d.tag(&need(a.tag.clone(), "tag", &act)?)?;
            let mut t = lf::PixmithTag { from: 0, to: 0, direction: 0, repeat: 0, color: 0 };
            unsafe { lf::pixmith_tag_get(raw, i, &mut t) };
            let name =
                a.name.clone().unwrap_or_else(|| unsafe { crate::doc::take_string(lf::pixmith_tag_name(raw, i)) });
            if let Some(f) = a.from {
                t.from = check(f)?;
            }
            if let Some(f) = a.to {
                t.to = check(f)?;
            }
            if t.from > t.to {
                std::mem::swap(&mut t.from, &mut t.to);
            }
            if let Some(dir) = a.direction {
                t.direction = dir as u8;
            }
            if let Some(r) = a.repeat {
                t.repeat = r;
            }
            if let Some(c) = a.color {
                t.color = c.0 >> 8;
            }
            let c = cstr(&name);
            d.edit("Tag Properties", Undo::Step, |_| {
                unsafe { lf::pixmith_tag_update(raw, i, c.as_ptr(), t) }
                    .then_some(())
                    .ok_or_else(|| fail("could not update the tag"))
            })?;
        }
        TagAction::Remove => {
            let i = d.tag(&need(a.tag.clone(), "tag", &act)?)?;
            d.edit("Delete Tag", Undo::Step, |_| {
                unsafe { lf::pixmith_tag_remove(raw, i) }.then_some(()).ok_or_else(|| fail("could not remove the tag"))
            })?;
        }
    }
    Out::json(json!({ "doc": d.id, "tags": tags_json(d) }))
}
