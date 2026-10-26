//! Palette editing, palette files and presets, and color mode conversion.

use super::docs::{mode_name, palette, preset_index, preset_names};
use super::{CHANGE, Out, ToolDef, need, tool};
use crate::color::{Color, hex};
use crate::doc::{cstr, fail};
use crate::engine::{Engine, R, Undo};
use crate::paths::show;
use pixmith_core::document::color_mode as cm;
use pixmith_core::ffi::document as core;
use pixmith_core::palette::{edit as pe, io as pio, presets};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Color",
            "palette",
            "Palette",
            CHANGE,
            PaletteArgs,
            palette_tool,
            "Read or edit the document palette (0-based indices; one undo step per call). Actions: get, set (entry `index` to `color`; in indexed mode pixels using it follow), add (append `colors`), insert (`color` at `index`), remove (`indices`; alias: delete), move (`indices` to before `to`), duplicate (`indices`), replace_all (the palette becomes `colors`), sort (by hue or luminance), ramp (insert `steps` blended colors between entries `index` and `to`, in RGB or HSV), from_sprite (palette of the colors used, optionally quantized to max_colors), list_presets, load_preset (`name`), load_file / save_file (`path`, GIMP .gpl or .hex)."
        ),
        tool!(
            "Color",
            "color_mode",
            "Color mode",
            CHANGE,
            ModeArgs,
            color_mode,
            "Convert the sprite's color mode: rgba, grayscale, or indexed (every pixel snapped to the palette; `dither` 2/4/8 uses an ordered Bayer dither instead of nearest color). conform_only=true re-snaps all pixels to the current mode without converting (after editing the palette, say)."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PaletteAction {
    Get,
    Set,
    Add,
    Insert,
    #[serde(alias = "delete")]
    Remove,
    Move,
    Duplicate,
    ReplaceAll,
    Sort,
    Ramp,
    FromSprite,
    ListPresets,
    LoadPreset,
    LoadFile,
    SaveFile,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    Hue,
    Luminance,
}

#[derive(Deserialize, JsonSchema)]
pub struct PaletteArgs {
    pub action: PaletteAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// set/insert: entry index; ramp: first entry.
    pub index: Option<u32>,
    /// set/insert: the color.
    pub color: Option<Color>,
    /// add/replace_all: colors.
    pub colors: Option<Vec<Color>>,
    /// remove/move/duplicate: entry indices.
    pub indices: Option<Vec<u32>>,
    /// move: destination (entries go before it; = palette size appends); ramp: second entry.
    pub to: Option<u32>,
    /// ramp: colors to insert (1-256).
    pub steps: Option<u32>,
    /// ramp: blend in HSV instead of RGB.
    pub hsv: Option<bool>,
    /// sort: hue (default) or luminance.
    pub by: Option<SortBy>,
    /// from_sprite: quantize to at most this many colors (default: exact, up to 256).
    pub max_colors: Option<u32>,
    /// load_preset: preset name.
    pub name: Option<String>,
    /// load_file/save_file: .gpl or .hex file.
    pub path: Option<String>,
}

fn palette_tool(e: &mut Engine, a: PaletteArgs) -> R<Out> {
    let act = super::act(&a.action);
    if a.action == PaletteAction::ListPresets {
        return Out::json(json!({ "presets": preset_names() }));
    }
    let file = match a.action {
        PaletteAction::LoadFile => Some(e.sandbox.read(need(a.path.as_deref(), "path", &act)?)?),
        PaletteAction::SaveFile => Some(e.sandbox.write(need(a.path.as_deref(), "path", &act)?)?),
        _ => None,
    };
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let count = || unsafe { core::pixmith_palette_count(raw) };
    let check = |i: u32| {
        if i < count() { Ok(i) } else { Err(format!("palette entry {i} does not exist ({} entries)", count())) }
    };
    let ok = |b: bool, what: &str| if b { Ok(()) } else { Err(fail(what.to_string())) };
    match a.action {
        PaletteAction::Get | PaletteAction::ListPresets => {}
        PaletteAction::Set => {
            let i = need(a.index, "index", &act)?;
            let c = need(a.color, "color", &act)?;
            if i > count() {
                return Err(format!("index {i} is past the end ({} entries; index = size appends)", count()));
            }
            d.edit("Edit Palette Color", Undo::Step, |_| {
                unsafe { cm::pixmith_cm_palette_set(raw, i, c.0) };
                Ok(())
            })?;
        }
        PaletteAction::Add => {
            let colors = need(a.colors.as_ref(), "colors", &act)?;
            d.edit("Add Palette Colors", Undo::Step, |_| {
                for c in colors {
                    unsafe { core::pixmith_palette_add(raw, c.0) };
                }
                Ok(())
            })?;
        }
        PaletteAction::Insert => {
            let i = need(a.index, "index", &act)?;
            let c = need(a.color, "color", &act)?;
            d.edit("Insert Palette Color", Undo::Step, |_| {
                ok(unsafe { pe::pixmith_pe_insert(raw, i, c.0) }, "could not insert")
            })?;
        }
        PaletteAction::Remove | PaletteAction::Duplicate | PaletteAction::Move => {
            let idx = need(a.indices.as_ref(), "indices", &act)?;
            for &i in idx {
                check(i)?;
            }
            let n = idx.len() as u32;
            match a.action {
                PaletteAction::Remove => d.edit("Remove Palette Colors", Undo::Step, |_| {
                    ok(
                        unsafe { pe::pixmith_pe_remove(raw, idx.as_ptr(), n) },
                        "could not remove (a palette needs one color)",
                    )
                })?,
                PaletteAction::Duplicate => d.edit("Duplicate Palette Colors", Undo::Step, |_| {
                    ok(unsafe { pe::pixmith_pe_duplicate(raw, idx.as_ptr(), n) } > 0, "could not duplicate")
                })?,
                _ => {
                    let to = need(a.to, "to", &act)?;
                    d.edit("Move Palette Colors", Undo::Step, |_| {
                        ok(unsafe { pe::pixmith_pe_move(raw, idx.as_ptr(), n, to) } >= 0, "nothing moved")
                    })?
                }
            }
        }
        PaletteAction::ReplaceAll => {
            let colors = need(a.colors.as_ref(), "colors", &act)?;
            if colors.is_empty() || colors.len() > 256 {
                return Err("replace_all needs 1-256 colors".into());
            }
            d.edit("Replace Palette", Undo::Step, |_| {
                for (i, c) in colors.iter().enumerate() {
                    unsafe { cm::pixmith_cm_palette_set(raw, i as u32, c.0) };
                }
                let extra: Vec<u32> = (colors.len() as u32..count()).collect();
                if !extra.is_empty() {
                    ok(
                        unsafe { pe::pixmith_pe_remove(raw, extra.as_ptr(), extra.len() as u32) },
                        "could not trim the palette",
                    )?;
                }
                Ok(())
            })?;
        }
        PaletteAction::Sort => {
            let by = a.by.unwrap_or(SortBy::Hue);
            d.edit(
                if by == SortBy::Hue { "Sort Palette by Hue" } else { "Sort Palette by Luminance" },
                Undo::Step,
                |_| {
                    unsafe { pio::pixmith_palette_sort(raw, by as u32) };
                    Ok(())
                },
            )?;
        }
        PaletteAction::Ramp => {
            let from = check(need(a.index, "index", &act)?)?;
            let to = check(need(a.to, "to", &act)?)?;
            let steps = a.steps.unwrap_or(4).clamp(1, 256);
            d.edit("Color Ramp", Undo::Step, |_| {
                ok(
                    unsafe { pe::pixmith_pe_ramp(raw, from, to, steps, a.hsv.unwrap_or(false)) } >= 0,
                    "could not make the ramp",
                )
            })?;
        }
        PaletteAction::FromSprite => {
            let max = a.max_colors.unwrap_or(0).min(256);
            d.edit("Palette from Sprite", Undo::Step, |_| {
                ok(unsafe { cm::pixmith_cm_palette_from_sprite(raw, max) } > 0, "the sprite has no visible colors")
            })?;
        }
        PaletteAction::LoadPreset => {
            let i = preset_index(need(a.name.as_deref(), "name", &act)?)?;
            d.edit("Load Palette Preset", Undo::Step, |_| {
                ok(unsafe { presets::pixmith_pe_preset_load(raw, i) }, "could not load the preset")
            })?;
        }
        PaletteAction::LoadFile => {
            let c = cstr(&file.as_ref().expect("path").to_string_lossy());
            d.edit("Load Palette", Undo::Step, |_| {
                ok(unsafe { pio::pixmith_palette_load(raw, c.as_ptr()) }, "could not load the palette")
            })?;
        }
        PaletteAction::SaveFile => {
            let p = file.as_ref().expect("path");
            let c = cstr(&p.to_string_lossy());
            crate::doc::clear_error();
            ok(unsafe { pio::pixmith_palette_save(raw, c.as_ptr()) }, "could not save the palette")?;
            return Out::json(json!({ "doc": d.id, "saved": show(p), "colors": count() }));
        }
    }
    let colors: Vec<String> = palette(d).iter().map(|&c| hex(c)).collect();
    Out::json(json!({ "doc": d.id, "size": colors.len(), "palette": colors }))
}

#[derive(Deserialize, JsonSchema)]
pub struct ModeArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Target mode (required unless conform_only).
    pub mode: Option<super::docs::ColorMode>,
    /// indexed: 0 nearest color (default), or Bayer matrix size 2, 4, 8.
    pub dither: Option<u32>,
    /// Only re-snap all pixels to the current mode.
    pub conform_only: Option<bool>,
}

fn color_mode(e: &mut Engine, a: ModeArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let mut changed = true;
    if a.conform_only.unwrap_or(false) {
        changed = d.edit_changed("Conform Colors", |_| Ok(unsafe { cm::pixmith_cm_conform_all(raw) }))?;
    } else {
        let m = a.mode.ok_or("give `mode` (rgba, grayscale, indexed) or conform_only=true")?;
        let dither = a.dither.unwrap_or(0);
        if ![0, 2, 4, 8].contains(&dither) {
            return Err("dither must be 0, 2, 4 or 8".into());
        }
        if unsafe { cm::pixmith_cm_get_mode(raw) } == m.code() {
            changed = false;
        } else {
            d.edit("Convert Color Mode", Undo::Step, |_| {
                unsafe { cm::pixmith_cm_convert(raw, m.code(), dither) }
                    .then_some(())
                    .ok_or_else(|| fail("could not convert"))
            })?;
        }
    }
    Out::json(json!({
        "doc": d.id, "color_mode": mode_name(unsafe { cm::pixmith_cm_get_mode(raw) }), "changed": changed,
        "transparent_index": unsafe { cm::pixmith_cm_transparent_index(raw) },
    }))
}
