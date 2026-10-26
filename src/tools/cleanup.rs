//! Cleaning up sprites made from reference art: Sprite Lint (read-only report) and Trace Pixel Art (pseudo-pixel
//! art from AI image tools onto a real pixel grid). Clean Edges and Remove Background are kinds of `filter`.

use super::filters::Target;
use super::view::{auto_scale, parse_background};
use super::{CHANGE, Out, READ, ToolDef, tool};
use crate::color::Color;
use crate::doc::{Doc, clear_error, cstr, fail, take_string};
use crate::engine::{Engine, R, Ref};
use crate::image::Img;
use pixmith_core::Sprite;
use pixmith_core::document::Cel;
use pixmith_core::formats::json as cjson;
use pixmith_core::paint::cleanup::{self, LintList, LintOptions};
use pixmith_core::paint::filters::Target as CoreTarget;
use pixmith_core::paint::trace::{self, TraceOptions, TraceResult};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Cleanup",
            "lint",
            "Sprite lint",
            READ,
            LintArgs,
            lint,
            "FX > Lint Sprite…: inspect a cel (default), every frame of a layer (target=layer) or every layer of a frame (target=frame) and report problems without changing anything. Per cel and category a count (always complete) and a capped list of positions {x, y, size} (0-based pixel coordinates; for an area its first pixel in scan order and its pixel count). Categories: light_edge (light, grayish paper-halo pixels on the two outermost rings), holes (enclosed transparent areas), specks (small pieces detached from the body, `speck` pixels or fewer), no_outline (edge pixels that are not outline-colored and have no outline neighbor), strays (a pixel unlike all 8 neighbors), semi_transparent (0 < alpha < 255), off_palette (colors missing from the palette; only with palette=true). Fix halos, specks and thick outlines with `filter` kind clean_edges."
        ),
        tool!(
            "Cleanup",
            "trace",
            "Trace pixel art onto a grid",
            CHANGE,
            TraceArgs,
            trace,
            "File > Import > Trace Pixel Art… on a cel of an open document: \"pseudo-pixel\" art (AI image tools: upscaled soft edges, a drifting grid, noise, paper-white background, drop shadow) becomes real pixel art. Finds the grid pitch and phase (pitch auto, or a number / [x, y] in source pixels), takes the median color of each cell, snaps to a palette (colors: a count, \"auto\", \"keep\", \"palette\" = the sprite's palette, or a list of colors), optionally removes the background and shadow (remove_background, shadow, tolerance, global, background_color), closes 1 px breaks in the dark outline (close_outline) and crops to the opaque cells (crop, default true). The chosen cel defines the grid, palette and crop and every image cel of the document is resampled with them, so an animation stays on one grid; the canvas becomes the traced size (one undo step). Returns width, height, pitch, phase, palette, quality (0-1), origin, grid_cells, closed_outline and the background report. preview=true returns the traced picture as an image, detect_only=true only the pitch and phase; neither changes anything. To trace straight from a file into a new document use doc_open format=trace_grid. Needs an RGB sprite without tilemap layers."
        ),
    ]
}

// MARK: lint

#[derive(Deserialize, JsonSchema)]
pub struct LintArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// cel (default): one cel; layer: every frame of the layer; frame: every layer of the frame.
    pub target: Option<Target>,
    /// Layer (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// Pieces of this many pixels or fewer that are detached from the body count as specks (default 4; 0 = off).
    pub speck: Option<u32>,
    /// Positions listed per category and cel, 0-1000 (default 64). The counts are always complete.
    pub list: Option<u32>,
    /// Also report colors that are not in the palette (off_palette).
    pub palette: Option<bool>,
    /// The outline color (default: pixels at most outline_luma bright count as outline).
    pub outline_color: Option<Color>,
    /// Largest per-channel difference from outline_color that still counts as outline, 0-255 (default 0).
    pub outline_tolerance: Option<u32>,
    /// Darkness limit of an automatically detected outline, luma 0-255 (default 70).
    pub outline_luma: Option<u32>,
}

/// Cels listed at most in one lint result (the summary still counts all of them).
const MAX_CELS: usize = 200;

fn list_json(l: &LintList) -> Value {
    json!({ "count": l.count, "items": l.items.iter().map(|i| json!({ "x": i.x, "y": i.y, "size": i.size })).collect::<Vec<_>>() })
}

fn lint(e: &mut Engine, a: LintArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let target = a.target.unwrap_or(Target::Cel);
    let layer = match target {
        Target::Frame => d.layer(&a.layer)?,
        _ => d.image_layer(&a.layer)?,
    };
    let frame = d.frame(a.frame)?;
    let mut o = LintOptions::default();
    if let Some(v) = a.speck {
        o.speck_size = v.min(1024);
    }
    if let Some(v) = a.list {
        o.max_list = v.min(1000) as usize;
    }
    o.check_palette = a.palette.unwrap_or(false);
    if let Some(c) = a.outline_color {
        let b = c.0.to_be_bytes();
        o.outline.color = Some([b[0], b[1], b[2]]);
        o.outline.tolerance = a.outline_tolerance.unwrap_or(0).min(255);
    }
    if let Some(l) = a.outline_luma {
        o.outline.max_luma = l.min(255);
    }
    let core_target = match target {
        Target::Cel => CoreTarget::Cel,
        Target::Layer => CoreTarget::AllFrames,
        Target::Frame => CoreTarget::AllLayers,
    };
    let sprite = d.doc.sprite();
    let cels = cleanup::lint_sprite(sprite, &o, core_target, layer as usize, frame as usize);
    let mut by_category = serde_json::Map::new();
    let mut total = 0usize;
    let mut listed = Vec::new();
    for c in &cels {
        total += c.report.total();
        for (name, l) in c.report.categories() {
            let n = by_category.entry(name.to_string()).or_insert(json!(0));
            *n = json!(n.as_u64().unwrap_or(0) + l.count as u64);
        }
        if listed.len() < MAX_CELS {
            let mut v = serde_json::Map::new();
            v.insert("layer".into(), json!(c.layer));
            v.insert("frame".into(), json!(c.frame));
            v.insert("layer_name".into(), json!(sprite.layers[c.layer].name));
            v.insert("total".into(), json!(c.report.total()));
            for (name, l) in c.report.categories() {
                v.insert(name.into(), list_json(l));
            }
            listed.push(Value::Object(v));
        }
    }
    let mut out = json!({
        "doc": d.id, "target": super::act(&target), "cels_checked": cels.len(), "total": total,
        "totals_by_category": by_category, "cels": listed,
    });
    if cels.len() > MAX_CELS {
        out["cels_omitted"] = json!(cels.len() - MAX_CELS);
    }
    if cels.is_empty() {
        out["note"] = json!("no cel with pixels was checked (groups and empty cels are skipped)");
    }
    Out::json(out)
}

// MARK: trace

#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum PitchArg {
    /// "auto" (find it between min_pitch and max_pitch).
    Auto(String),
    /// One cell size for both axes, in source pixels (fractions such as 6.5 are fine).
    One(f64),
    /// [x, y] cell size in source pixels.
    Two([f64; 2]),
}

#[derive(Deserialize, JsonSchema, Clone, Debug)]
#[serde(untagged)]
pub enum ColorsArg {
    /// A palette of this many colors (k-means), 1-256.
    Count(u32),
    /// "auto" (the smallest count in min_colors..max_colors that fits, default 2..32), "keep" (no quantising) or "palette" (the sprite's current palette).
    Word(String),
    /// Snap to exactly these colors.
    List(Vec<Color>),
}

/// The options of a trace, shared by the `trace` tool and `doc_open` format trace_grid.
#[derive(Clone, Default)]
pub struct TraceOpts {
    pub pitch: Option<PitchArg>,
    pub min_pitch: Option<f64>,
    pub max_pitch: Option<f64>,
    pub phase: Option<[f64; 2]>,
    pub separate_axes: Option<bool>,
    pub colors: Option<ColorsArg>,
    pub min_colors: Option<u32>,
    pub max_colors: Option<u32>,
    pub remove_background: Option<bool>,
    pub tolerance: Option<u32>,
    pub shadow: Option<bool>,
    pub shadow_depth: Option<u32>,
    pub global: Option<bool>,
    pub background_color: Option<Color>,
    pub close_outline: Option<bool>,
    pub crop: Option<bool>,
    pub seed: Option<u64>,
}

fn hex6(c: Color) -> String {
    format!("#{:06x}", c.0 >> 8)
}

impl TraceOpts {
    /// The JSON options of the core (`pixmith_trace_grid`). `palette` is what `colors: "palette"` snaps to.
    pub fn json(&self, palette: &[u32]) -> R<Value> {
        let mut o = serde_json::Map::new();
        match &self.pitch {
            None => {}
            Some(PitchArg::Auto(s)) if s.trim().eq_ignore_ascii_case("auto") => {}
            Some(PitchArg::Auto(s)) => {
                let n: f64 =
                    s.trim().parse().map_err(|_| format!("pitch {s:?} must be \"auto\", a number or [x, y]"))?;
                o.insert("pitch".into(), json!(n));
            }
            Some(PitchArg::One(n)) => {
                o.insert("pitch".into(), json!(n));
            }
            Some(PitchArg::Two(p)) => {
                o.insert("pitch".into(), json!(p));
            }
        }
        for (k, v) in [("min_pitch", self.min_pitch), ("max_pitch", self.max_pitch)] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        if let Some(p) = self.phase {
            o.insert("phase".into(), json!(p));
        }
        if let Some(b) = self.separate_axes {
            o.insert("separate_axes".into(), json!(b));
        }
        match &self.colors {
            None => {}
            Some(ColorsArg::Count(n)) => {
                o.insert("colors".into(), json!(n));
            }
            Some(ColorsArg::Word(w)) if w.trim().eq_ignore_ascii_case("palette") => {
                if palette.is_empty() {
                    return Err("colors \"palette\": the sprite has no palette".into());
                }
                o.insert("colors".into(), json!(palette.iter().map(|&c| hex6(Color(c))).collect::<Vec<_>>()));
            }
            Some(ColorsArg::Word(w)) => {
                let w = w.trim().to_ascii_lowercase();
                if let Ok(n) = w.parse::<u32>() {
                    o.insert("colors".into(), json!(n));
                } else if w == "auto" || w == "keep" {
                    o.insert("colors".into(), json!(w));
                } else {
                    return Err(format!(
                        "colors {w:?} must be a count, \"auto\", \"keep\", \"palette\" or a list of colors"
                    ));
                }
            }
            Some(ColorsArg::List(l)) => {
                o.insert("colors".into(), json!(l.iter().map(|c| hex6(*c)).collect::<Vec<_>>()));
            }
        }
        for (k, v) in [("min_colors", self.min_colors), ("max_colors", self.max_colors)] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        // Naming any background option turns the removal on.
        let bg = self.remove_background.unwrap_or(
            self.shadow.is_some()
                || self.global.is_some()
                || self.tolerance.is_some()
                || self.shadow_depth.is_some()
                || self.background_color.is_some(),
        );
        o.insert("remove_background".into(), json!(bg));
        if let Some(v) = self.tolerance {
            o.insert("tolerance".into(), json!(v.min(255)));
        }
        if let Some(v) = self.shadow {
            o.insert("shadow".into(), json!(v));
        }
        if let Some(v) = self.shadow_depth {
            o.insert("shadow_depth".into(), json!(v.clamp(1, 95)));
        }
        if let Some(v) = self.global {
            o.insert("global".into(), json!(v));
        }
        if let Some(c) = self.background_color {
            o.insert("color".into(), json!(hex6(c)));
        }
        if let Some(v) = self.close_outline {
            o.insert("close_outline".into(), json!(v));
        }
        if let Some(v) = self.crop {
            o.insert("crop".into(), json!(v));
        }
        if let Some(v) = self.seed {
            o.insert("seed".into(), json!(v));
        }
        Ok(Value::Object(o))
    }

    fn core(&self, palette: &[u32]) -> R<TraceOptions> {
        let text = self.json(palette)?.to_string();
        let v = cjson::parse(&text)?;
        TraceOptions::from_json(&v)
    }
}

#[derive(Deserialize, JsonSchema)]
pub struct TraceArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// The cel that defines the grid, palette and crop: layer (default: active).
    pub layer: Option<Ref>,
    /// The cel that defines the grid: frame (default: active).
    pub frame: Option<u32>,
    /// Cell size in source pixels: "auto" (default), a number such as 6.5, or [x, y].
    pub pitch: Option<PitchArg>,
    /// pitch auto: smallest cell size searched (default 3).
    pub min_pitch: Option<f64>,
    /// pitch auto: largest cell size searched (default 32).
    pub max_pitch: Option<f64>,
    /// [x, y] offset of the first cell edge (default: detected).
    pub phase: Option<[f64; 2]>,
    /// Find the x and y pitch independently (default: one pitch for both, which is steadier).
    pub separate_axes: Option<bool>,
    /// Palette: a count, "auto" (default), "keep", "palette" (the sprite's palette) or a list of colors.
    pub colors: Option<ColorsArg>,
    /// colors auto: fewest colors tried (default 2).
    pub min_colors: Option<u32>,
    /// colors auto: most colors tried (default 32).
    pub max_colors: Option<u32>,
    /// Remove the background from the traced cells (also on when any of tolerance, shadow, shadow_depth, global or background_color is given).
    pub remove_background: Option<bool>,
    /// remove_background: per-channel tolerance 0-255 (default 28).
    pub tolerance: Option<u32>,
    /// remove_background: also remove a soft drop shadow.
    pub shadow: Option<bool>,
    /// remove_background: how much darker than the background a shadow may be, percent 1-95 (default 55).
    pub shadow_depth: Option<u32>,
    /// remove_background: also remove enclosed background-colored pixels.
    pub global: Option<bool>,
    /// remove_background: the background color (default: the dominant border color).
    pub background_color: Option<Color>,
    /// Close 1 px breaks in the dark outer outline.
    pub close_outline: Option<bool>,
    /// Crop to the opaque cells (default true).
    pub crop: Option<bool>,
    /// k-means seed (default 1; the same seed gives the same palette).
    pub seed: Option<u64>,
    /// Return the traced picture as an image and change nothing.
    pub preview: Option<bool>,
    /// Only find the grid (pitch and phase) and change nothing.
    pub detect_only: Option<bool>,
    /// preview: integer upscale (default auto).
    pub scale: Option<u32>,
    /// preview: "checker" (default), "transparent" or a color.
    pub background: Option<String>,
}

impl TraceArgs {
    fn opts(&self) -> TraceOpts {
        TraceOpts {
            pitch: self.pitch.clone(),
            min_pitch: self.min_pitch,
            max_pitch: self.max_pitch,
            phase: self.phase,
            separate_axes: self.separate_axes,
            colors: self.colors.clone(),
            min_colors: self.min_colors,
            max_colors: self.max_colors,
            remove_background: self.remove_background,
            tolerance: self.tolerance,
            shadow: self.shadow,
            shadow_depth: self.shadow_depth,
            global: self.global,
            background_color: self.background_color,
            close_outline: self.close_outline,
            crop: self.crop,
            seed: self.seed,
        }
    }
}

/// A report of the core (its own JSON type) as `serde_json`.
fn convert(v: &cjson::Value) -> Value {
    serde_json::from_str(&cjson::to_string(v)).unwrap_or(Value::Null)
}

fn trace(e: &mut Engine, a: TraceArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let layer = d.image_layer(&a.layer)?;
    let frame = d.frame(a.frame)?;
    let palette = super::docs::palette(d);
    let opts = a.opts();
    let options = opts.json(&palette)?;
    let (w, h) = (d.doc.width() as usize, d.doc.height() as usize);
    if a.preview.unwrap_or(false) || a.detect_only.unwrap_or(false) {
        let px = super::view::pixels(d, Some(layer), frame)?;
        let o = opts.core(&palette)?;
        if a.detect_only.unwrap_or(false) {
            let g = trace::detect_grid(&px, w, h, &o.pitch, o.separate_axes)?;
            return Out::json(json!({
                "doc": d.id, "layer": layer, "frame": frame, "applied": false,
                "pitch": [g.pitch.0, g.pitch.1], "phase": [g.phase.0, g.phase.1],
            }));
        }
        let r = trace::trace_grid(&px, w, h, &o)?;
        let scale = auto_scale(r.width, r.height, a.scale);
        let img = Img { w: r.width, h: r.height, px: r.rgba.clone() }
            .upscale(scale)
            .on_background(parse_background(&a.background)?, (scale * 4).clamp(4, 16));
        let mut report = convert(&r.to_json());
        report["applied"] = json!(false);
        report["scale"] = json!(scale);
        report["doc"] = json!(d.id);
        return Ok(Out { value: report, images: vec![img.png()?], text: None });
    }
    let raw = d.doc.raw();
    let c = cstr(&options.to_string());
    let report = d.edit("Trace Pixel Art", crate::engine::Undo::Step, |_| {
        clear_error();
        let p = unsafe { pixmith_core::paint::trace::pixmith_trace_grid(raw, layer, frame, c.as_ptr()) };
        if p.is_null() {
            return Err(fail("could not trace the cel"));
        }
        Ok(unsafe { take_string(p) })
    })?;
    d.clamp_active();
    let mut v: Value = serde_json::from_str(&report).unwrap_or(Value::Null);
    v["doc"] = json!(d.id);
    v["layer"] = json!(layer);
    v["frame"] = json!(frame);
    v["applied"] = json!(true);
    v["canvas"] = json!([d.doc.width(), d.doc.height()]);
    Out::json(v)
}

/// `doc_open` format trace_grid: the first image of the file traced onto its grid, as a new document. Returns the
/// document and the trace report.
pub fn open_traced(path: &str, opts: &TraceOpts) -> R<(Doc, Value)> {
    let c = cstr(path);
    clear_error();
    let ed = unsafe { pixmith_core::formats::raster::abi::pixmith_import_image_ex(c.as_ptr(), std::ptr::null()) };
    let src = Doc::adopt(ed, &format!("could not open {path}"))?;
    let (w, h) = (src.width() as usize, src.height() as usize);
    let rgba = src.render(0)?;
    let o = opts.core(&[])?;
    let r: TraceResult = trace::trace_grid(&rgba, w, h, &o)?;
    let mut s = Sprite::new(r.width as u32, r.height as u32);
    if !r.palette.is_empty() && r.palette.len() <= 256 {
        s.palette = r.palette.clone();
    }
    s.cels[0][0] = Some(Arc::new(Cel { opacity: 255, pixels: r.rgba.clone(), ..Default::default() }));
    let report = convert(&r.to_json());
    Ok((Doc::from_sprite(s), report))
}
