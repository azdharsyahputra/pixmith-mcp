//! Documents: create, open/import, save, close, list, inspect, and the default layer/frame.

use super::{ADD, OPEN_FILES, Out, READ, ToolDef, WRITE_FILES, tool};
use crate::color::{Color, hex, hex_rgb};
use crate::doc::{Doc, clear_error, cstr, fail, take_string};
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use crate::paths::show;
use pixmith_core::document::{color_mode as cm, layers as lf, slices as sf, tilemap as tm};
use pixmith_core::ffi::document as core;
use pixmith_core::formats::{animation, import, sheet_import};
use pixmith_core::gamedev::{autotile, rig, tween};
use pixmith_core::history;
use pixmith_core::paint::{selection as sel, transform as xf};
use pixmith_core::palette::presets;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Documents",
            "doc_new",
            "New document",
            ADD,
            NewArgs,
            doc_new,
            "Create a new blank sprite (one transparent layer \"Layer 1\", one 100 ms frame) and return its document id. Optionally fill it with a background color, pick the color mode, or load a built-in palette. Coordinates everywhere are pixels, 0-based from the top-left; layers and frames are 0-based (layer 0 = bottom of the stack)."
        ),
        tool!(
            "Documents",
            "doc_open",
            "Open or import a file",
            OPEN_FILES,
            OpenArgs,
            doc_open,
            "Open a file as a new document and return its id plus a summary. Reads Pixmith documents (.pxmt) and Aseprite files (.aseprite/.ase), and imports PNG (one frame), animated GIF (frames + delays), a sprite sheet PNG cut into a grid (give cell_width/cell_height), or a sprite sheet with Aseprite JSON metadata (pass the .json; the PNG named in meta.image is used unless png_path is given), or, with format=trace_grid (File > Import > Trace Pixel Art…), the first image of a file (PNG, JPEG, WebP, ...) traced onto its real pixel grid: \"pseudo-pixel\" art from AI image tools becomes real pixel art (options pitch auto|n|[x, y], colors, remove_background, shadow, tolerance, global, background_color, close_outline, crop, seed; the result's `trace` has pitch, phase, palette, quality). Imported documents have no .pxmt path yet; doc_save without `path` saves them next to the imported file (same name, .pxmt)."
        ),
        tool!(
            "Documents",
            "doc_save",
            "Save document",
            WRITE_FILES,
            SaveArgs,
            doc_save,
            "Save the document. Without `path` it saves to where it was opened/last saved; a document made by an import (PNG, GIF, sheet) and never saved goes next to the imported file (same name, .pxmt), and the result says where. replace=true first closes another open document (an app tab in live mode) that has the target file open, if it has no unsaved changes; with unsaved changes the call fails and nothing is closed. The format follows the extension: .pxmt (Pixmith document, keeps everything) or .aseprite/.ase. Use the export tool for PNG/GIF/sheets. With copy=true it writes a copy and keeps the document's own path and unsaved state."
        ),
        tool!(
            "Documents",
            "doc_close",
            "Close document",
            Hints2::CLOSE,
            CloseArgs,
            doc_close,
            "Close a document and free it. Refuses when it has unsaved changes unless discard_changes=true. With all_headless=true it instead closes every headless document (ids d1, d2, ...; never the Pixmith app's tabs) at once: documents with unsaved changes are kept and listed unless discard_changes=true. Headless documents also pile up on their own: when more than 32 are open, doc_new / doc_open close the oldest unmodified ones and say so in the result `note`."
        ),
        tool!(
            "Documents",
            "doc_list",
            "List open documents",
            READ,
            NoArgs,
            doc_list,
            "List the open documents: id, name, path, size, layer/frame counts, unsaved state."
        ),
        tool!(
            "Documents",
            "doc_info",
            "Document details",
            READ,
            InfoArgs,
            doc_info,
            "Everything about a document: size, color mode, layer tree (index 0 = bottom; kind image/group/tilemap, visibility, opacity, blend mode, tileset, iso grid and offset of isometric tilemaps, tween parent/terrain data, user data), frames and durations, which cels have pixels or are linked, tags, palette, slices, tilesets, sprite user data, selection, free-transform/floating state, undo history size and the active layer/frame defaults. Use `sections` to fetch only parts of it."
        ),
        tool!(
            "Documents",
            "doc_set_active",
            "Set active layer/frame",
            Hints2::ACTIVE,
            ActiveArgs,
            doc_set_active,
            "Set the document's active layer and/or frame: the defaults every tool uses when `layer`/`frame` are omitted (Lua scripts see them as activeLayer/activeFrame). New layers and frames become active automatically."
        ),
    ]
}

struct Hints2;
impl Hints2 {
    const CLOSE: super::Hints = super::Hints { read_only: false, destructive: true, idempotent: true, files: false };
    /// Changes only the server's per-document defaults.
    const ACTIVE: super::Hints = super::Hints { read_only: false, destructive: false, idempotent: true, files: false };
}

#[derive(Deserialize, JsonSchema)]
pub struct NoArgs {}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    Rgba,
    Grayscale,
    Indexed,
}

impl ColorMode {
    pub fn code(self) -> u32 {
        self as u32
    }
}

pub fn mode_name(m: u32) -> &'static str {
    ["rgba", "grayscale", "indexed"].get(m as usize).copied().unwrap_or("unknown")
}

#[derive(Deserialize, JsonSchema)]
pub struct NewArgs {
    /// Canvas width in pixels (1-16384).
    pub width: u32,
    /// Canvas height in pixels (1-16384).
    pub height: u32,
    /// Display name (default "Untitled N").
    pub name: Option<String>,
    /// Fill layer 1 with this color (default: transparent).
    pub background: Option<Color>,
    /// Color mode (default rgba). indexed snaps every color to the palette.
    pub color_mode: Option<ColorMode>,
    /// Load a built-in palette by name (see palette action list_presets), e.g. "PICO-8".
    pub palette_preset: Option<String>,
    /// "app" (default; the sprite opens in a new tab of the Pixmith app, which is started when it is not running, so
    /// the user sees it) or "headless" (kept inside this server only: only when the user asks for that).
    pub target: Option<Target>,
}

/// Where a new document lives (live mode).
#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    App,
    Headless,
}

pub fn doc_new(e: &mut Engine, a: NewArgs) -> R<Out> {
    if a.target == Some(Target::App) {
        return Err(e.live.not_connected());
    }
    if a.width == 0 || a.height == 0 || a.width > 16384 || a.height > 16384 {
        return Err("width and height must be 1-16384".into());
    }
    let doc = Doc::new(a.width, a.height)?;
    let name = a.name.clone().unwrap_or_else(|| format!("Untitled {}", e.docs.len() + 1));
    let id = e.add(doc, name, None, None);
    let entry = e.doc(&Some(id.clone()))?;
    let r = (|| {
        if let Some(p) = &a.palette_preset {
            let i = preset_index(p)?;
            entry.edit("Load Palette Preset", Undo::Step, |d| {
                unsafe { presets::pixmith_pe_preset_load(d.doc.raw(), i) }
                    .then_some(())
                    .ok_or_else(|| fail("could not load the preset"))
            })?;
        }
        if let Some(bg) = a.background.filter(|c| c.0 & 0xFF != 0) {
            entry.edit("Fill", Undo::Step, |d| {
                unsafe { core::pixmith_fill(d.doc.raw(), 0, 0, 0, 0, bg.0) };
                Ok(())
            })?;
        }
        if let Some(m) = a.color_mode.filter(|m| *m != ColorMode::Rgba) {
            entry.edit("Convert Color Mode", Undo::Step, |d| {
                unsafe { cm::pixmith_cm_convert(d.doc.raw(), m.code(), 0) }
                    .then_some(())
                    .ok_or_else(|| fail("could not convert the color mode"))
            })?;
        }
        // A fresh document counts as unmodified; its setup steps can still be undone.
        entry.doc.mark_saved();
        Ok::<_, String>(())
    })();
    if let Err(err) = r {
        e.docs.retain(|d| d.id != id);
        return Err(err);
    }
    let entry = e.doc(&Some(id))?;
    Out::json(summary(entry))
}

pub fn preset_names() -> Vec<String> {
    (0..presets::pixmith_pe_preset_count())
        .map(|i| unsafe { take_string(presets::pixmith_pe_preset_name(i)) })
        .collect()
}

pub fn preset_index(name: &str) -> R<u32> {
    let names = preset_names();
    let norm = |s: &str| s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
    names
        .iter()
        .position(|n| norm(n) == norm(name))
        .map(|i| i as u32)
        .ok_or_else(|| format!("no palette preset {name:?}; presets: {}", names.join(", ")))
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum OpenFormat {
    Auto,
    Native,
    Png,
    Gif,
    SheetGrid,
    SheetJson,
    /// File > Import > Trace Pixel Art…: pseudo-pixel art (AI image tools) traced onto its real pixel grid.
    TraceGrid,
}

#[derive(Deserialize, JsonSchema)]
pub struct OpenArgs {
    /// File to open: .pxmt, .aseprite, .ase, .png, .gif, or a sprite-sheet .json (Aseprite format); with format trace_grid any image.
    pub path: String,
    /// How to read it (default auto: by extension; .png with cell_width becomes sheet_grid). native = .pxmt/.aseprite/.ase.
    pub format: Option<OpenFormat>,
    /// sheet_grid: frame width in pixels.
    pub cell_width: Option<u32>,
    /// sheet_grid: frame height in pixels (default = cell_width).
    pub cell_height: Option<u32>,
    /// sheet_json: the sheet image (default: meta.image from the JSON, next to it).
    pub png_path: Option<String>,
    /// "app" (default; a tab of the Pixmith app, started when it is not running) or "headless" (kept inside this
    /// server only: only when the user asks for that).
    pub target: Option<Target>,
    /// trace_grid: cell size in source pixels: "auto" (default), a number such as 6.5, or [x, y].
    pub pitch: Option<super::cleanup::PitchArg>,
    /// trace_grid, pitch auto: smallest and largest cell size searched (default 3 and 32).
    pub min_pitch: Option<f64>,
    pub max_pitch: Option<f64>,
    /// trace_grid: [x, y] offset of the first cell edge (default: detected).
    pub phase: Option<[f64; 2]>,
    /// trace_grid: find the x and y pitch independently.
    pub separate_axes: Option<bool>,
    /// trace_grid: palette: a count, "auto" (default), "keep" or a list of colors.
    pub colors: Option<super::cleanup::ColorsArg>,
    /// trace_grid: remove the background (also on when tolerance, shadow, global or background_color is given).
    pub remove_background: Option<bool>,
    /// trace_grid, remove_background: per-channel tolerance 0-255 (default 28).
    pub tolerance: Option<u32>,
    /// trace_grid, remove_background: also remove a soft drop shadow.
    pub shadow: Option<bool>,
    /// trace_grid, remove_background: also remove enclosed background-colored pixels.
    pub global: Option<bool>,
    /// trace_grid, remove_background: the background color (default: the dominant border color).
    pub background_color: Option<Color>,
    /// trace_grid: close 1 px breaks in the dark outer outline.
    pub close_outline: Option<bool>,
    /// trace_grid: crop to the opaque cells (default true).
    pub crop: Option<bool>,
    /// trace_grid: k-means seed (default 1).
    pub seed: Option<u64>,
}

impl OpenArgs {
    /// The trace options of format trace_grid.
    pub fn trace_opts(&self) -> super::cleanup::TraceOpts {
        super::cleanup::TraceOpts {
            pitch: self.pitch.clone(),
            min_pitch: self.min_pitch,
            max_pitch: self.max_pitch,
            phase: self.phase,
            separate_axes: self.separate_axes,
            colors: self.colors.clone(),
            remove_background: self.remove_background,
            tolerance: self.tolerance,
            shadow: self.shadow,
            global: self.global,
            background_color: self.background_color,
            close_outline: self.close_outline,
            crop: self.crop,
            seed: self.seed,
            ..Default::default()
        }
    }
}

/// How `doc_open` reads a file with this (lower-case) extension.
pub fn open_format(a: &OpenArgs, ext: &str) -> OpenFormat {
    match a.format.unwrap_or(OpenFormat::Auto) {
        OpenFormat::Auto => match ext {
            "png" if a.cell_width.is_some() => OpenFormat::SheetGrid,
            "png" => OpenFormat::Png,
            "gif" => OpenFormat::Gif,
            "json" => OpenFormat::SheetJson,
            _ => OpenFormat::Native,
        },
        f => f,
    }
}

pub fn doc_open(e: &mut Engine, a: OpenArgs) -> R<Out> {
    if a.target == Some(Target::App) {
        return Err(e.live.not_connected());
    }
    let path = e.sandbox.read(&a.path)?;
    if path.is_dir() {
        return Err(format!("{}: is a directory", a.path));
    }
    let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let format = open_format(&a, &ext);
    let c = cstr(&path.to_string_lossy());
    clear_error();
    let what = format!("could not open {}", a.path);
    let mut trace = None;
    let (doc, native) = match format {
        OpenFormat::TraceGrid => {
            let (doc, report) = super::cleanup::open_traced(&path.to_string_lossy(), &a.trace_opts())
                .map_err(|m| format!("{}: {m}", a.path))?;
            trace = Some(report);
            (doc, false)
        }
        OpenFormat::Native | OpenFormat::Auto => (Doc::adopt(unsafe { core::pixmith_open(c.as_ptr()) }, &what)?, true),
        OpenFormat::Png => (Doc::adopt(unsafe { import::pixmith_import_png(c.as_ptr()) }, &what)?, false),
        OpenFormat::Gif => (Doc::adopt(unsafe { animation::pixmith_import_gif(c.as_ptr()) }, &what)?, false),
        OpenFormat::SheetGrid => {
            let w = a.cell_width.ok_or("sheet_grid needs cell_width")?;
            let h = a.cell_height.unwrap_or(w);
            (Doc::adopt(unsafe { import::pixmith_import_sheet(c.as_ptr(), w, h) }, &what)?, false)
        }
        OpenFormat::SheetJson => {
            let png = match &a.png_path {
                Some(p) => e.sandbox.read(p)?,
                None => sheet_image(&path, &e.sandbox)?,
            };
            let p = cstr(&png.to_string_lossy());
            (Doc::adopt(unsafe { sheet_import::pixmith_import_sheet_json(c.as_ptr(), p.as_ptr()) }, &what)?, false)
        }
    };
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Untitled".into());
    let (p, src) = if native { (Some(path.clone()), None) } else { (None, Some(path.clone())) };
    let id = e.add(doc, name, p, src);
    let entry = e.doc(&Some(id))?;
    entry.doc.mark_saved();
    let mut v = summary(entry);
    if let Some(t) = trace {
        v["trace"] = t;
    }
    Out::json(v)
}

/// The image a sprite-sheet JSON names in `meta.image`, resolved next to the JSON and checked against the root.
fn sheet_image(json: &Path, sandbox: &crate::paths::Sandbox) -> R<PathBuf> {
    let text = std::fs::read_to_string(json).map_err(|e| format!("{}: {e}", json.display()))?;
    let v = pixmith_core::formats::json::parse(&text).map_err(|e| format!("{}: {e}", json.display()))?;
    let image = v
        .get("meta")
        .and_then(|m| m.get("image"))
        .and_then(|i| i.as_str())
        .ok_or("the JSON has no meta.image; pass png_path")?;
    let p = json.parent().unwrap_or(Path::new(".")).join(image);
    sandbox.read(&p.to_string_lossy())
}

#[derive(Deserialize, JsonSchema)]
pub struct SaveArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Destination (.pxmt, .aseprite or .ase). Default: the document's current path; for a document made by an
    /// import (PNG, GIF, sheet) that was never saved: next to the imported file, same name, .pxmt.
    pub path: Option<String>,
    /// Write a copy only: the document keeps its path and unsaved state.
    pub copy: Option<bool>,
    /// Live mode: the Pixmith app refuses to save to a file another tab has open. With replace=true that other tab
    /// (or headless document) is closed first, but only when it has no unsaved changes (otherwise the call fails and
    /// nothing is closed).
    pub replace: Option<bool>,
}

/// Where an imported document is saved when no path is given: next to its source, same name, `.pxmt`.
pub fn default_save_path(source: &Path) -> PathBuf {
    source.with_extension("pxmt")
}

fn doc_save(e: &mut Engine, a: SaveArgs) -> R<Out> {
    let sandbox = e.sandbox.clone();
    let d = e.doc(&a.doc)?;
    let target = match (&a.path, &d.path) {
        (Some(p), _) => sandbox.write(p)?,
        (None, Some(p)) => sandbox.write(&p.to_string_lossy())?,
        (None, None) => match &d.source {
            Some(src) => sandbox.write(&default_save_path(src).to_string_lossy())?,
            None => {
                return Err(format!(
                    "document {} has no file yet; pass path (e.g. \"{}.pxmt\" or \"{}.aseprite\")",
                    d.id, d.name, d.name
                ));
            }
        },
    };
    let defaulted = a.path.is_none() && d.path.is_none();
    let ext = target.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "pxmt" | "aseprite" | "ase") {
        return Err(format!(
            "{}: save writes .pxmt, .aseprite or .ase; use the export tool for .{ext} files",
            show(&target)
        ));
    }
    let (self_id, copy_only) = (d.id.clone(), a.copy.unwrap_or(false));
    // replace=true: another headless document that has this file open and no unsaved changes is closed first;
    // one with unsaved changes stops the save.
    let mut closed_other = Vec::new();
    if a.replace.unwrap_or(false) && !copy_only {
        let holders: Vec<(String, bool)> = e
            .docs
            .iter()
            .filter(|o| o.id != self_id && o.path.as_deref() == Some(target.as_path()))
            .map(|o| (o.id.clone(), o.doc.is_dirty()))
            .collect();
        if let Some((id, _)) = holders.iter().find(|(_, dirty)| *dirty) {
            return Err(format!(
                "{} is open as {id} with unsaved changes, so replace did not close it and nothing was saved. Save it elsewhere or close it with discard_changes=true, then retry",
                show(&target)
            ));
        }
        for (id, _) in holders {
            e.docs.retain(|o| o.id != id);
            closed_other.push(id);
        }
    }
    let d = e.doc(&Some(self_id))?;
    let c = cstr(&target.to_string_lossy());
    clear_error();
    if !unsafe { core::pixmith_save(d.doc.raw(), c.as_ptr()) } {
        return Err(fail(format!("could not save {}", show(&target))));
    }
    let copy = a.copy.unwrap_or(false);
    if !copy {
        d.doc.mark_saved();
        d.path = Some(target.clone());
        if let Some(stem) = target.file_stem() {
            d.name = stem.to_string_lossy().into_owned();
        }
    }
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let mut v = json!({ "doc": d.id, "saved": show(&target), "format": ext, "bytes": size, "copy": copy, "dirty": d.doc.is_dirty() });
    if defaulted {
        v["note"] = json!(format!("no path given: saved next to the imported file as {}", show(&target)));
    }
    if !closed_other.is_empty() {
        v["closed_documents"] = json!(closed_other);
    }
    Out::json(v)
}

#[derive(Deserialize, JsonSchema)]
pub struct CloseArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Close even if there are unsaved changes (they are lost).
    pub discard_changes: Option<bool>,
    /// Close every headless document (d1, d2, ...) instead of one; the app's documents are never touched. Documents
    /// with unsaved changes stay open (and are listed in `kept_unsaved`) unless `discard_changes` is true. Not
    /// combinable with `doc`.
    pub all_headless: Option<bool>,
}

/// Most headless documents kept open before `doc_new` / `doc_open` close the oldest unmodified ones.
pub const MAX_HEADLESS_DOCS: usize = 32;

fn doc_close(e: &mut Engine, a: CloseArgs) -> R<Out> {
    if a.all_headless.unwrap_or(false) {
        if a.doc.is_some() {
            return Err("give either `doc` or `all_headless`, not both".into());
        }
        let discard = a.discard_changes.unwrap_or(false);
        let (mut closed, mut kept) = (Vec::new(), Vec::new());
        e.docs.retain(|d| {
            if d.id.starts_with("app:") {
                return true;
            }
            if d.doc.is_dirty() && !discard {
                kept.push(d.id.clone());
                return true;
            }
            closed.push(d.id.clone());
            false
        });
        let open: Vec<String> = e.docs.iter().map(|d| d.id.clone()).collect();
        let mut v = json!({ "closed": closed, "kept_unsaved": kept, "open": open });
        if !kept.is_empty() {
            v["note"] = json!(format!(
                "{} headless document(s) have unsaved changes and stay open; save them with doc_save or pass discard_changes=true",
                kept.len()
            ));
        }
        return Out::json(v);
    }
    let d = e.doc(&a.doc)?;
    if d.doc.is_dirty() && !a.discard_changes.unwrap_or(false) {
        return Err(format!(
            "document {} ({}) has unsaved changes; save it with doc_save or pass discard_changes=true",
            d.id, d.name
        ));
    }
    let id = d.id.clone();
    e.docs.retain(|x| x.id != id);
    Out::json(json!({ "closed": id, "open": e.docs.iter().map(|d| d.id.clone()).collect::<Vec<_>>() }))
}

fn doc_list(e: &mut Engine, _: NoArgs) -> R<Out> {
    let list: Vec<Value> = e.docs.iter().map(summary).collect();
    Out::json(json!({ "documents": list, "root": e.sandbox.root().map(show) }))
}

/// Short description of a document.
pub fn summary(d: &DocEntry) -> Value {
    let raw = d.doc.raw();
    json!({
        "doc": d.id,
        "name": d.name,
        "path": d.path.as_deref().map(show),
        "imported_from": d.source.as_deref().map(show),
        "width": d.doc.width(),
        "height": d.doc.height(),
        "color_mode": mode_name(unsafe { cm::pixmith_cm_get_mode(raw) }),
        "layers": d.layer_count(),
        "frames": d.doc.frame_count(),
        "tags": unsafe { lf::pixmith_tag_count(raw) },
        "dirty": d.doc.is_dirty(),
        "active_layer": d.layer,
        "active_frame": d.frame,
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Layers,
    Frames,
    Cels,
    Tags,
    Palette,
    Slices,
    Tilesets,
    UserData,
    Selection,
    History,
}

#[derive(Deserialize, JsonSchema)]
pub struct InfoArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Parts to include (default: all). The summary (size, mode, counts) is always included.
    pub sections: Option<Vec<Section>>,
}

fn doc_info(e: &mut Engine, a: InfoArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let want = |s: Section| a.sections.as_ref().is_none_or(|v| v.contains(&s));
    let mut out = summary(d);
    let raw = d.doc.raw();
    let o = out.as_object_mut().expect("summary is an object");
    o.insert("transparent_index".into(), json!(unsafe { cm::pixmith_cm_transparent_index(raw) }));
    if want(Section::Layers) {
        o.insert("layers".into(), Value::Array(layers_json(d)));
    }
    if want(Section::Frames) {
        let n = d.doc.frame_count();
        let durations: Vec<u16> = (0..n).map(|f| unsafe { core::pixmith_frame_duration(raw, f) }).collect();
        let total: u64 = durations.iter().map(|&x| x as u64).sum();
        let shown = &durations[..durations.len().min(512)];
        o.insert(
            "frame_durations_ms".into(),
            json!({ "count": n, "total_ms": total, "durations": shown, "truncated": durations.len() > shown.len() }),
        );
    }
    if want(Section::Cels) {
        o.insert("cels".into(), cels_json(d));
    }
    if want(Section::Tags) {
        o.insert("tag_list".into(), Value::Array(tags_json(d)));
    }
    if want(Section::Palette) {
        o.insert("palette".into(), json!(palette(d).iter().map(|&c| hex(c)).collect::<Vec<_>>()));
    }
    if want(Section::Slices) {
        o.insert("slices".into(), Value::Array(super::slices::slices_json(d, d.frame)));
    }
    if want(Section::Tilesets) {
        o.insert("tilesets".into(), Value::Array(tilesets_json(d)));
    }
    if want(Section::UserData) {
        o.insert("sprite_user_data".into(), user_data(d, 0, 0, 0));
    }
    if want(Section::Selection) {
        o.insert("selection".into(), selection_json(d));
    }
    if want(Section::History) {
        let (undo, redo) = d.doc.history_counts();
        o.insert(
            "history".into(),
            json!({ "undo_steps": undo, "redo_steps": redo, "open_transactions": d.txns.len(),
                    "undo_bytes": unsafe { history::pixmith_undo_bytes(raw) } }),
        );
    }
    Out::json(out)
}

pub const BLEND_NAMES: [&str; 19] = [
    "normal",
    "multiply",
    "screen",
    "overlay",
    "darken",
    "lighten",
    "color_dodge",
    "color_burn",
    "hard_light",
    "soft_light",
    "difference",
    "exclusion",
    "hue",
    "saturation",
    "color",
    "luminosity",
    "addition",
    "subtract",
    "divide",
];

pub fn layers_json(d: &DocEntry) -> Vec<Value> {
    let raw = d.doc.raw();
    let n = d.layer_count();
    let levels: Vec<u32> = (0..n).map(|i| unsafe { core::pixmith_layer_level(raw, i) }).collect();
    (0..n)
        .map(|i| {
            let group = d.is_group(i);
            let tilemap = d.is_tilemap(i);
            let mut parent = None;
            if levels[i as usize] > 0 {
                // A group comes before (below) its children in the layer list.
                parent = (0..i).rev().find(|&j| levels[j as usize] + 1 == levels[i as usize] && d.is_group(j));
            }
            let blend = unsafe { lf::pixmith_layer_blend_mode(raw, i) };
            let mut l = json!({
                "index": i,
                "name": d.layer_name(i),
                "kind": if group { "group" } else if tilemap { "tilemap" } else { "image" },
                "level": levels[i as usize],
                "parent": parent,
                "visible": unsafe { core::pixmith_layer_visible(raw, i) },
                "opacity": unsafe { core::pixmith_layer_opacity(raw, i) },
                "blend_mode": BLEND_NAMES.get(blend as usize).copied().unwrap_or("normal"),
            });
            let m = l.as_object_mut().expect("object");
            if tilemap {
                m.insert("tileset".into(), json!(unsafe { tm::pixmith_layer_tileset(raw, i) }));
                let mode = unsafe { autotile::pixmith_autotile_mode(raw, i) };
                if mode >= 0 {
                    m.insert("terrain_mode".into(), json!(super::tilemap::TERRAIN_MODES[mode as usize % 3]));
                }
                let iso = super::tilemap::layer_iso_json(d, i);
                if !iso.is_null() {
                    m.insert("iso".into(), iso);
                }
            }
            let parent = unsafe { rig::pixmith_tween_get_parent(raw, i) };
            if parent >= 0 {
                m.insert("tween_parent".into(), json!(parent));
            }
            let tw = unsafe { tween::pixmith_tween_options(raw, i) };
            if tw & 1 != 0 {
                m.insert("tweened".into(), json!(true));
            }
            let ud = user_data(d, 1, i, 0);
            if !ud.is_null() {
                m.insert("user_data".into(), ud);
            }
            if i == d.layer {
                m.insert("active".into(), json!(true));
            }
            l
        })
        .collect()
}

/// Cel occupancy per layer: one character per frame. `#` pixels, `o` empty cel, `.` no cel, `L` linked.
fn cels_json(d: &DocEntry) -> Value {
    let (layers, frames) = (d.layer_count() as usize, d.doc.frame_count() as usize);
    if layers * frames > 65536 {
        return json!({ "omitted": "too many cels; use get_pixels / render per cel" });
    }
    let mut grid = vec![0u8; layers * frames];
    if !unsafe { lf::pixmith_cel_grid(d.doc.raw(), grid.as_mut_ptr(), grid.len()) } {
        return Value::Null;
    }
    let rows: Vec<Value> = (0..layers)
        .map(|l| {
            let s: String = (0..frames)
                .map(|f| {
                    let b = grid[l * frames + f];
                    if b & 4 != 0 {
                        'L'
                    } else if b & 2 != 0 {
                        '#'
                    } else if b & 1 != 0 {
                        'o'
                    } else {
                        '.'
                    }
                })
                .collect();
            json!({ "layer": l, "frames": s })
        })
        .collect();
    json!({ "legend": "one char per frame: # has pixels, o empty cel, . no cel, L linked to another frame", "rows": rows })
}

pub const DIRECTIONS: [&str; 4] = ["forward", "reverse", "ping_pong", "ping_pong_reverse"];

pub fn tags_json(d: &DocEntry) -> Vec<Value> {
    let raw = d.doc.raw();
    let n = unsafe { lf::pixmith_tag_count(raw) };
    (0..n)
        .filter_map(|i| {
            let mut t = lf::PixmithTag { from: 0, to: 0, direction: 0, repeat: 0, color: 0 };
            unsafe { lf::pixmith_tag_get(raw, i, &mut t) }.then(|| {
                json!({
                    "index": i,
                    "name": unsafe { take_string(lf::pixmith_tag_name(raw, i)) },
                    "from": t.from, "to": t.to,
                    "direction": DIRECTIONS.get(t.direction as usize).copied().unwrap_or("forward"),
                    "repeat": t.repeat,
                    "color": hex_rgb(t.color),
                })
            })
        })
        .collect()
}

pub fn palette(d: &DocEntry) -> Vec<u32> {
    let raw = d.doc.raw();
    let n = unsafe { core::pixmith_palette_count(raw) };
    (0..n).map(|i| unsafe { core::pixmith_palette_color(raw, i) }).collect()
}

pub fn tilesets_json(d: &DocEntry) -> Vec<Value> {
    let raw = d.doc.raw();
    let n = unsafe { tm::pixmith_tileset_count(raw) };
    (0..n)
        .map(|t| {
            let users: Vec<u32> = (0..d.layer_count())
                .filter(|&l| d.is_tilemap(l) && unsafe { tm::pixmith_layer_tileset(raw, l) } == t as i32)
                .collect();
            json!({
                "index": t,
                "name": unsafe { take_string(tm::pixmith_tileset_name(raw, t)) },
                "tile_width": unsafe { tm::pixmith_tileset_tile_width(raw, t) },
                "tile_height": unsafe { tm::pixmith_tileset_tile_height(raw, t) },
                "tiles": unsafe { tm::pixmith_tileset_tile_count(raw, t) },
                "base_index": unsafe { tm::pixmith_tileset_base_index(raw, t) },
                "used_by_layers": users,
                "iso": super::tilemap::tileset_iso_json(d, t),
            })
        })
        .collect()
}

/// User data of a target (kind 0 sprite, 1 layer, 2 cel (a=frame, b=layer), 3 tag, 4 slice), or null if empty.
pub fn user_data(d: &DocEntry, kind: u32, a: u32, b: u32) -> Value {
    let raw = d.doc.raw();
    let text = unsafe { take_string(sf::pixmith_userdata_text(raw, kind, a, b)) };
    let mut c = 0u32;
    let has_color = unsafe { sf::pixmith_userdata_color(raw, kind, a, b, &mut c) };
    // Pixmith keeps its own data (terrains, tweens, export settings) in user data lines; hide those.
    let visible: Vec<&str> =
        text.lines().filter(|l| !l.starts_with("pixmith.") && !l.starts_with("pixmith-")).collect();
    let text = visible.join("\n");
    if text.is_empty() && !has_color {
        return Value::Null;
    }
    json!({ "text": (!text.is_empty()).then_some(text), "color": has_color.then(|| hex(c)) })
}

pub fn selection_json(d: &DocEntry) -> Value {
    let raw = d.doc.raw();
    let mut b = [0i32; 4];
    let active = unsafe { sel::pixmith_sel_active(raw) } && unsafe { sel::pixmith_sel_bounds(raw, b.as_mut_ptr()) };
    json!({
        "active": active,
        "bounds": active.then(|| json!({ "x": b[0], "y": b[1], "w": b[2], "h": b[3] })),
        "floating_paste": unsafe { sel::pixmith_float_active(raw) },
        "free_transform": unsafe { xf::pixmith_xf_active(raw) },
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct ActiveArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer index or name to make active.
    pub layer: Option<Ref>,
    /// Frame index to make active.
    pub frame: Option<u32>,
}

fn doc_set_active(e: &mut Engine, a: ActiveArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    if a.layer.is_some() {
        d.layer = d.layer(&a.layer)?;
    }
    if a.frame.is_some() {
        d.frame = d.frame(a.frame)?;
    }
    Out::json(
        json!({ "doc": d.id, "active_layer": d.layer, "layer_name": d.layer_name(d.layer), "active_frame": d.frame }),
    )
}
