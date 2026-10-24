//! Exports: PNG, GIF, APNG, PNG sequences and sprite sheets through the core's export planner (the same
//! options as the app's Export dialog), and the Godot 4 SpriteFrames and TileSet exporters.

use super::{Out, ToolDef, WRITE_FILES, tool};
use crate::color::Color;
use crate::doc::{clear_error, cstr, fail, take_string};
use crate::engine::{Engine, R, Ref};
use crate::paths::show;
use pixmith_core::formats::export_plan::{self, PixmithExport};
use pixmith_core::gamedev::godot;
use pixmith_core::gamedev::godot::tileset as gts;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Export",
            "export",
            "Export image / animation / sheet",
            WRITE_FILES,
            ExportArgs,
            export,
            "Export to image files (the document itself is not changed; use doc_save for .pxmt/.aseprite). format: png (one frame, `frame` counts within the exported frames), gif and apng (animated, frame durations kept), png_sequence (`path` is a base name; writes <base>-001.png, ...), sheet (sprite sheet PNG; layout horizontal / vertical / grid with `columns`, padding, border, extrude, json=true also writes Aseprite-style JSON metadata next to it). Options: scale (integer upscale), tag (export only that tag's frames, apply_direction expands reverse/ping-pong), split_tags (one file per tag), layers (visible / single `layer` / each layer to its own file), trim (none / per_frame / uniform), ignore_empty. dry_run=true only reports how many files it would write."
        ),
        tool!(
            "Export",
            "export_godot",
            "Export Godot SpriteFrames",
            WRITE_FILES,
            GodotArgs,
            export_godot,
            "Export a Godot 4 SpriteFrames resource (.tres, one animation per tag, durations kept) plus its atlas PNG(s), optionally an AnimatedSprite2D scene (.tscn), into a Godot project. `project_root` is the folder containing project.godot and `tres_path` must be inside it. Re-exporting keeps resource uids stable. Without project_root/tres_path the settings saved in the document are used (save_settings=true stores the ones you pass). preview=true writes nothing and returns the .tres text, the files it would write and the first atlas page as an image. Live Link: export once with save_settings=true so the document knows its res:// resource; after that a game running the Pixmith Live Link addon (godot/addons/pixmith_live_link in the Pixmith repository, debug builds only) updates its sprites after every edit, including edits made through this server, with no re-export or restart. Call game_screenshot to look at the running game. Live Link and the game view need the Pixmith app 0.5.1 or later."
        ),
        tool!(
            "Export",
            "export_godot_tileset",
            "Export Godot TileSet",
            WRITE_FILES,
            TilesetArgs,
            export_godot_tileset,
            "Export a tileset as a Godot 4 TileSet resource (.tres) with its atlas PNG, including the terrain set and peering bits from terrain auto-tiling (or `mode` when there is none), flipped alternatives and optional collision shapes; write_scene=true also writes a .tscn with TileMapLayer nodes painted from the document's tilemap layers. preview=true describes the export without writing."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    Png,
    Gif,
    Apng,
    Sheet,
    PngSequence,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Layers {
    Visible,
    Single,
    Each,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Trim {
    None,
    PerFrame,
    Uniform,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    Horizontal,
    Vertical,
    Grid,
}

#[derive(Deserialize, JsonSchema)]
pub struct ExportArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    pub format: Format,
    /// Output file (png_sequence: base name without extension). Split exports add "-<layer>-<tag>" to the name.
    pub path: String,
    /// Integer upscale 1-64 (default 1).
    pub scale: Option<u32>,
    /// png: which exported frame (default 0).
    pub frame: Option<u32>,
    /// Only this tag's frames (index or name).
    pub tag: Option<Ref>,
    /// Expand the tag direction (reverse, ping-pong) into the frame order (default true).
    pub apply_direction: Option<bool>,
    /// One output per tag.
    pub split_tags: Option<bool>,
    /// visible (default): layers as shown; single: only `layer`; each: every visible layer to its own file.
    pub layers: Option<Layers>,
    /// layers=single: the layer (default: active).
    pub layer: Option<Ref>,
    /// none (default), per_frame (crop each frame to its content), uniform (one crop for all frames).
    pub trim: Option<Trim>,
    /// Skip frames with no visible pixels.
    pub ignore_empty: Option<bool>,
    /// sheet: horizontal (default), vertical or grid.
    pub layout: Option<Layout>,
    /// sheet grid: columns (default: about square).
    pub columns: Option<u32>,
    /// sheet: transparent pixels between frames.
    pub padding: Option<u32>,
    /// sheet: margin around the sheet (default = padding).
    pub border: Option<u32>,
    /// sheet: repeat each frame's edge pixels this many times (against texture bleeding).
    pub extrude: Option<u32>,
    /// sheet: also write the Aseprite JSON metadata (<name>.json).
    pub json: Option<bool>,
    /// Only report how many files would be written.
    pub dry_run: Option<bool>,
}

/// Files in `base`'s directory whose names start with its stem, written since `since`.
fn written_files(base: &Path, since: SystemTime) -> Vec<String> {
    let (Some(dir), Some(stem)) = (base.parent(), base.file_stem().or(base.file_name())) else { return vec![] };
    let stem = stem.to_string_lossy().into_owned();
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(&stem))
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since))
        .map(|e| show(&e.path()))
        .filter(|p| !p.ends_with(".pixmith-tmp"))
        .collect();
    out.sort();
    out.truncate(500);
    out
}

fn export(e: &mut Engine, a: ExportArgs) -> R<Out> {
    let path = e.sandbox.write(&a.path)?;
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let tag = match &a.tag {
        Some(t) => d.tag(t)? as i32,
        None => -1,
    };
    let layers = a.layers.unwrap_or(Layers::Visible);
    let layer = if layers == Layers::Single { d.image_layer(&a.layer)? } else { 0 };
    let o = PixmithExport {
        format: a.format as u32,
        scale: a.scale.unwrap_or(1).clamp(1, 64),
        tag,
        apply_direction: a.apply_direction.unwrap_or(true),
        layers: layers as u32,
        layer,
        split_tags: a.split_tags.unwrap_or(false),
        trim: a.trim.map_or(0, |t| t as u32),
        ignore_empty: a.ignore_empty.unwrap_or(false),
        frame: a.frame.unwrap_or(0),
        layout: a.layout.map_or(0, |l| l as u32),
        columns: a.columns.unwrap_or(0),
        padding: a.padding.unwrap_or(0),
        border: a.border.unwrap_or(a.padding.unwrap_or(0)),
        extrude: a.extrude.unwrap_or(0),
        write_json: a.json.unwrap_or(false),
    };
    clear_error();
    let jobs = unsafe { export_plan::pixmith_export_job_count(raw, &o) };
    if jobs < 0 {
        return Err(fail("these export options are invalid"));
    }
    if a.dry_run.unwrap_or(false) {
        return Out::json(json!({ "doc": d.id, "outputs": jobs, "dry_run": true }));
    }
    let since = SystemTime::now() - std::time::Duration::from_secs(1);
    let c = cstr(&path.to_string_lossy());
    clear_error();
    let n = unsafe { export_plan::pixmith_export_run(raw, &o, c.as_ptr()) };
    if n < 0 {
        return Err(fail(format!("could not export {}", show(&path))));
    }
    Out::json(json!({ "doc": d.id, "files_written": n, "files": written_files(&path, since) }))
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Packing {
    Tight,
    Grid,
}

#[derive(Deserialize, JsonSchema)]
pub struct GodotArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Godot project folder (contains project.godot). Default: the document's saved settings.
    pub project_root: Option<String>,
    /// Output .tres path inside the project. Default: the document's saved settings.
    pub tres_path: Option<String>,
    /// Pixels between atlas frames.
    pub padding: Option<u32>,
    /// Repeat edge pixels into the padding.
    pub extrude: Option<bool>,
    /// Crop frames to their content (offsets keep them in place).
    pub trim: Option<bool>,
    /// Largest atlas page side (default 4096, max 16384).
    pub max_texture: Option<u32>,
    /// Integer upscale 1-16.
    pub scale: Option<u32>,
    /// tight (default) or grid.
    pub packing: Option<Packing>,
    /// One PNG per animation instead of one shared atlas.
    pub per_animation_png: Option<bool>,
    /// File stem template: {name} is the .tres stem, {layer} the layer name.
    pub name_template: Option<String>,
    /// Layers to include by name (default: visible layers).
    pub layers: Option<Vec<String>>,
    /// One SpriteFrames per visible layer.
    pub each_layer: Option<bool>,
    /// Only these tags.
    pub tags_include: Option<Vec<String>>,
    /// Skip these tags.
    pub tags_exclude: Option<Vec<String>>,
    /// Also write an AnimatedSprite2D scene (.tscn).
    pub tscn: Option<bool>,
    /// Overwrite an existing .tscn.
    pub overwrite_tscn: Option<bool>,
    /// Store these settings in the document (saved with it) for later re-exports.
    pub save_settings: Option<bool>,
    /// Describe the export without writing anything.
    pub preview: Option<bool>,
}

fn export_godot(e: &mut Engine, a: GodotArgs) -> R<Out> {
    let sandbox = e.sandbox.clone();
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let stored: Option<Value> = {
        let s = unsafe { godot::pixmith_godot_settings_get(raw) };
        (!s.is_null()).then(|| unsafe { take_string(s) }).and_then(|t| serde_json::from_str(&t).ok())
    };
    let stored_str = |k: &str| stored.as_ref().and_then(|v| v.get(k)).and_then(Value::as_str).map(str::to_string);
    let root = a
        .project_root
        .clone()
        .or_else(|| stored_str("root"))
        .ok_or("give project_root (the folder with project.godot)")?;
    let tres =
        a.tres_path.clone().or_else(|| stored_str("tres")).ok_or("give tres_path (e.g. <project>/art/player.tres)")?;
    let root = sandbox.dir(&root)?;
    let tres = if Path::new(&tres).is_absolute() { PathBuf::from(&tres) } else { root.join(&tres) };
    let tres = sandbox.write(&tres.to_string_lossy())?;
    let mut s = stored.clone().filter(Value::is_object).unwrap_or_else(|| json!({}));
    let set = |s: &mut Value, k: &str, v: Value| {
        s[k] = v;
    };
    set(&mut s, "v", json!(1));
    set(&mut s, "root", json!(root.to_string_lossy()));
    set(&mut s, "tres", json!(tres.to_string_lossy()));
    if let Some(v) = a.padding {
        set(&mut s, "padding", json!(v));
    }
    if let Some(v) = a.extrude {
        set(&mut s, "extrude", json!(v));
    }
    if let Some(v) = a.trim {
        set(&mut s, "trim", json!(v));
    }
    if let Some(v) = a.max_texture {
        set(&mut s, "max_texture", json!(v.min(16384)));
    }
    if let Some(v) = a.scale {
        set(&mut s, "scale", json!(v.clamp(1, 16)));
    }
    if let Some(v) = a.packing {
        set(&mut s, "packing", json!(if v == Packing::Grid { "grid" } else { "tight" }));
    }
    if let Some(v) = a.per_animation_png {
        set(&mut s, "per_animation_png", json!(v));
    }
    if let Some(v) = &a.name_template {
        set(&mut s, "name_template", json!(v));
    }
    if a.each_layer.unwrap_or(false) {
        set(&mut s, "layers", json!("each"));
    } else if let Some(v) = &a.layers {
        set(&mut s, "layers", json!(v));
    }
    if let Some(v) = &a.tags_include {
        set(&mut s, "tags_include", json!(v));
    }
    if let Some(v) = &a.tags_exclude {
        set(&mut s, "tags_exclude", json!(v));
    }
    if let Some(v) = a.tscn {
        set(&mut s, "tscn", json!(v));
    }
    if let Some(v) = a.overwrite_tscn {
        set(&mut s, "overwrite_tscn", json!(v));
    }
    let settings = s.to_string();
    let c = cstr(&settings);
    if a.preview.unwrap_or(false) {
        let tmp = std::env::temp_dir().join(format!("pixmith-mcp-{}-godot-preview.png", std::process::id()));
        let tp = cstr(&tmp.to_string_lossy());
        let (mut t, mut f, mut w) = (std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
        clear_error();
        let ok = unsafe { godot::pixmith_godot_preview(raw, c.as_ptr(), tp.as_ptr(), &mut t, &mut f, &mut w) };
        let (tres_text, files, warnings) = unsafe { (take_string(t), take_string(f), take_string(w)) };
        if !ok {
            let _ = std::fs::remove_file(&tmp);
            return Err(fail("the Godot export would fail"));
        }
        let image = std::fs::read(&tmp).ok();
        let _ = std::fs::remove_file(&tmp);
        let text: String = tres_text.chars().take(20000).collect();
        return Ok(Out {
            value: json!({ "doc": d.id, "preview": true, "files": lines(&files), "warnings": lines(&warnings), "tres": text }),
            images: image.into_iter().collect(),
            text: None,
        });
    }
    if a.save_settings.unwrap_or(false) {
        d.edit("Godot Export Settings", crate::engine::Undo::Step, |_| {
            unsafe { godot::pixmith_godot_settings_set(raw, c.as_ptr()) }
                .then_some(())
                .ok_or_else(|| fail("could not store the settings"))
        })?;
    }
    let since = SystemTime::now() - std::time::Duration::from_secs(1);
    let mut w = std::ptr::null_mut();
    clear_error();
    let ok = unsafe { godot::pixmith_export_godot(raw, c.as_ptr(), &mut w) };
    let warnings = unsafe { take_string(w) };
    if !ok {
        return Err(fail("the Godot export failed"));
    }
    let dir = tres.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut files: Vec<String> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t >= since))
        .map(|e| show(&e.path()))
        .collect();
    files.sort();
    Out::json(
        json!({ "doc": d.id, "tres": show(&tres), "files": files, "warnings": lines(&warnings), "settings_saved": a.save_settings.unwrap_or(false) }),
    )
}

fn lines(s: &str) -> Vec<String> {
    s.lines().filter(|l| !l.is_empty()).map(str::to_string).collect()
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GodotMode {
    Blob,
    Corners,
    Sides,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Collision {
    None,
    Full,
    Polygon,
}

#[derive(Deserialize, JsonSchema)]
pub struct TilesetArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Tileset index or name (default 0).
    pub tileset: Option<Ref>,
    /// Godot project folder (contains project.godot).
    pub project_root: String,
    /// Output .tres inside the project (relative paths are relative to project_root).
    pub tres_path: String,
    /// Terrain mode when the layer has no auto-tile data: blob (default), corners, sides.
    pub mode: Option<GodotMode>,
    /// none (default), full (tile square) or polygon (traced from opaque pixels).
    pub collision: Option<Collision>,
    pub margin: Option<u32>,
    pub separation: Option<u32>,
    /// Edge pixels to replicate around each tile.
    pub extrude: Option<u32>,
    /// Atlas page size limit (default 4096).
    pub max_texture: Option<u32>,
    /// Also write <name>.tscn with TileMapLayer nodes painted from the tilemap layers.
    pub write_scene: Option<bool>,
    /// Terrain name when there is no auto-tile data (default: the tileset name).
    pub terrain_name: Option<String>,
    /// Terrain color when there is no auto-tile data.
    pub terrain_color: Option<Color>,
    /// Describe the export without writing.
    pub preview: Option<bool>,
}

fn export_godot_tileset(e: &mut Engine, a: TilesetArgs) -> R<Out> {
    let sandbox = e.sandbox.clone();
    let root = sandbox.dir(&a.project_root)?;
    let tres =
        if Path::new(&a.tres_path).is_absolute() { PathBuf::from(&a.tres_path) } else { root.join(&a.tres_path) };
    let tres = sandbox.write(&tres.to_string_lossy())?;
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let ts = match &a.tileset {
        Some(r) => d.tileset(r)?,
        None => {
            if unsafe { pixmith_core::document::tilemap::pixmith_tileset_count(raw) } == 0 {
                return Err("the document has no tilesets".into());
            }
            0
        }
    };
    let name = cstr(a.terrain_name.as_deref().unwrap_or(""));
    let col = a.terrain_color.map_or(0x4CB04CFF, |c| c.0).to_be_bytes();
    let opts = gts::PixmithGodotOptions {
        mode: a.mode.map_or(0, |m| m as u32),
        collision: a.collision.map_or(0, |c| c as u32),
        margin: a.margin.unwrap_or(0),
        separation: a.separation.unwrap_or(0),
        extrude: a.extrude.unwrap_or(0),
        max_texture: a.max_texture.unwrap_or(0),
        write_scene: a.write_scene.unwrap_or(false),
        terrain_color: [col[0] as f32 / 255.0, col[1] as f32 / 255.0, col[2] as f32 / 255.0, col[3] as f32 / 255.0],
        terrain_name: name.as_ptr(),
    };
    let (r, t) = (cstr(&root.to_string_lossy()), cstr(&tres.to_string_lossy()));
    clear_error();
    if a.preview.unwrap_or(false) {
        let p = unsafe { gts::pixmith_godot_tileset_preview(raw, ts, r.as_ptr(), t.as_ptr(), &opts) };
        if p.is_null() {
            return Err(fail("the TileSet export would fail"));
        }
        return Out::json(json!({ "doc": d.id, "preview": true, "description": unsafe { take_string(p) } }));
    }
    let since = SystemTime::now() - std::time::Duration::from_secs(1);
    if !unsafe { gts::pixmith_export_godot_tileset_ex(raw, ts, r.as_ptr(), t.as_ptr(), &opts) } {
        return Err(fail("the TileSet export failed"));
    }
    let warnings = take_string_safe(gts::pixmith_godot_tileset_warning());
    Out::json(
        json!({ "doc": d.id, "tileset": ts, "tres": show(&tres), "files": written_files(&tres, since), "warnings": lines(&warnings) }),
    )
}

fn take_string_safe(p: *mut std::ffi::c_char) -> String {
    // SAFETY: the core returns null or a string it allocated.
    unsafe { take_string(p) }
}
