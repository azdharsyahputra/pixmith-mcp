//! FX filters, applied as one undo step (or previewed as an image without changing anything).

use super::view::{Rect, auto_scale, clip, parse_background};
use super::{CHANGE, Out, ToolDef, tool};
use crate::color::Color;
use crate::doc::{clear_error, fail};
use crate::engine::{Engine, R, Ref};
use crate::image::Img;
use pixmith_core::paint::background::{self, BgOptions};
use pixmith_core::paint::{filters, selection as sel};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Filters",
        "filter",
        "Apply FX filter",
        CHANGE,
        FilterArgs,
        filter,
        "Apply an FX filter to a cel (default), every frame of a layer (target=layer) or every layer of a frame (target=frame), limited to the selection's bounds when something is selected or to `region`. Kinds and their parameters: replace_color (from_color, to_color, tolerance), invert, desaturate, hsl (hue -180..180, saturation and lightness -100..100), brightness_contrast (brightness, contrast -100..100), color_balance (red, green, blue offsets -255..255), outline (color, inside, eight_connected), drop_shadow (dx, dy, color), remove_isolated_pixels (eight_connected), blur, sharpen (3x3, alpha-aware), curves (curve_rgb / curve_red / curve_green / curve_blue / curve_alpha: [[x, y], ...] control points in 0-255), clean_edges (FX > Clean Edges…: tidy a sprite traced from reference art, never touching interior detail such as eyes, 1 px highlights or fingers; passes halo [halo_rings, halo_gain, halo_saturation], despeckle [pieces smaller than N pixels that touch the outside], holes [fill enclosed transparent areas smaller than N], light_specks, thin_outline [thickness, outline_color, outline_tolerance, outline_luma]; name no pass to run halo + despeckle + thin_outline with their defaults, otherwise only the passes you name run; use the read-only `lint` tool to see what is wrong first), remove_background (FX > Remove Background…: make the paper background transparent by flood fill from the border: tolerance 0-255 default 28, shadow [also a soft drop shadow, shadow_depth 1-95 percent], global [also enclosed background-colored pixels], color [background color; default the dominant border color]). preview=true returns an image of the result and changes nothing; with `region` the image is cropped to that region (the result says so)."
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    ReplaceColor,
    Invert,
    Desaturate,
    Hsl,
    BrightnessContrast,
    ColorBalance,
    Outline,
    DropShadow,
    RemoveIsolatedPixels,
    Blur,
    Sharpen,
    Curves,
    /// FX > Clean Edges…
    CleanEdges,
    /// FX > Remove Background…
    RemoveBackground,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Cel,
    Layer,
    Frame,
}

#[derive(Deserialize, JsonSchema)]
pub struct FilterArgs {
    pub kind: Kind,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// cel (default): one cel; layer: every frame of the layer; frame: every layer of the frame.
    pub target: Option<Target>,
    /// Layer (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// Only change pixels inside this rectangle (default: the selection bounds, or the whole canvas).
    pub region: Option<Rect>,
    /// Return a preview image instead of applying (cropped to `region` when given).
    pub preview: Option<bool>,
    /// replace_color: color to replace.
    pub from_color: Option<Color>,
    /// replace_color: replacement.
    pub to_color: Option<Color>,
    /// replace_color: 0-255 (default 0); remove_background: 0-255 per channel (default 28).
    pub tolerance: Option<i32>,
    /// hsl: degrees -180..180.
    pub hue: Option<i32>,
    /// hsl: -100..100.
    pub saturation: Option<i32>,
    /// hsl: -100..100.
    pub lightness: Option<i32>,
    /// brightness_contrast: -100..100.
    pub brightness: Option<i32>,
    /// brightness_contrast: -100..100.
    pub contrast: Option<i32>,
    /// color_balance: -255..255.
    pub red: Option<i32>,
    /// color_balance: -255..255.
    pub green: Option<i32>,
    /// color_balance: -255..255.
    pub blue: Option<i32>,
    /// outline/drop_shadow: color (default black); remove_background: the background color (default: the dominant border color).
    pub color: Option<Color>,
    /// remove_background: also remove a soft drop shadow connected to the background.
    pub shadow: Option<bool>,
    /// remove_background: how much darker than the background a shadow may be, percent 1-95 (default 55).
    pub shadow_depth: Option<i32>,
    /// remove_background: also remove background-colored pixels enclosed by the figure (eyes, windows).
    pub global: Option<bool>,
    /// clean_edges halo pass: remove light, grayish paper-halo pixels from the outer rings (true, or give halo_rings).
    pub halo: Option<bool>,
    /// clean_edges: rings to peel, 1-8 (default 2); names the halo pass.
    pub halo_rings: Option<u32>,
    /// clean_edges: how much lighter than what is behind a halo pixel must be, luma 0-255 (default 40); names the halo pass.
    pub halo_gain: Option<u32>,
    /// clean_edges: halo pixels are at most this saturated, percent 0-100 (default 30); names the halo pass.
    pub halo_saturation: Option<u32>,
    /// clean_edges despeckle pass: remove loose pieces smaller than this many pixels that touch the outside (default 4; 0 = off).
    pub despeckle: Option<u32>,
    /// clean_edges: fill enclosed transparent areas smaller than this many pixels (0 = off; a transparent eye is a hole too).
    pub holes: Option<u32>,
    /// clean_edges: also replace a lone light pixel whose 8 neighbors are all much darker (a 1 px highlight in a dark eye looks like this).
    pub light_specks: Option<bool>,
    /// clean_edges: thin a dark outer outline of 2..thickness pixels to 1 pixel (true, or give thickness).
    pub thin_outline: Option<bool>,
    /// clean_edges: thickest outline that is thinned, 0-16 (default 3); names the thin_outline pass.
    pub thickness: Option<u32>,
    /// clean_edges: the outline color (default: pixels at most outline_luma bright are outline).
    pub outline_color: Option<Color>,
    /// clean_edges: largest per-channel difference from outline_color, 0-255 (default 0).
    pub outline_tolerance: Option<u32>,
    /// clean_edges: darkness limit of an automatically detected outline, luma 0-255 (default 70).
    pub outline_luma: Option<u32>,
    /// outline: draw on the shape's edge pixels instead of around it.
    pub inside: Option<bool>,
    /// outline/remove_isolated_pixels: count diagonal neighbors.
    pub eight_connected: Option<bool>,
    /// drop_shadow: offset (default 1).
    pub dx: Option<i32>,
    /// drop_shadow: offset (default 1).
    pub dy: Option<i32>,
    /// curves: master RGB curve points [[x, y], ...].
    pub curve_rgb: Option<Vec<[i32; 2]>>,
    pub curve_red: Option<Vec<[i32; 2]>>,
    pub curve_green: Option<Vec<[i32; 2]>>,
    pub curve_blue: Option<Vec<[i32; 2]>>,
    pub curve_alpha: Option<Vec<[i32; 2]>>,
    /// preview: integer upscale of the image (default auto).
    pub scale: Option<u32>,
    /// preview: "checker" (default), "transparent" or a color.
    pub background: Option<String>,
}

fn params(a: &FilterArgs) -> R<Vec<i32>> {
    let c = |c: Option<Color>, d: u32| c.map_or(d, |c| c.0) as i32;
    let b = |v: Option<bool>| v.unwrap_or(false) as i32;
    Ok(match a.kind {
        Kind::ReplaceColor => vec![
            c(Some(a.from_color.ok_or("replace_color needs from_color")?), 0),
            c(Some(a.to_color.ok_or("replace_color needs to_color")?), 0),
            a.tolerance.unwrap_or(0).clamp(0, 255),
        ],
        Kind::Invert | Kind::Desaturate | Kind::Blur | Kind::Sharpen => vec![],
        Kind::Hsl => vec![
            a.hue.unwrap_or(0).clamp(-180, 180),
            a.saturation.unwrap_or(0).clamp(-100, 100),
            a.lightness.unwrap_or(0).clamp(-100, 100),
        ],
        Kind::BrightnessContrast => {
            vec![a.brightness.unwrap_or(0).clamp(-100, 100), a.contrast.unwrap_or(0).clamp(-100, 100)]
        }
        Kind::ColorBalance => vec![
            a.red.unwrap_or(0).clamp(-255, 255),
            a.green.unwrap_or(0).clamp(-255, 255),
            a.blue.unwrap_or(0).clamp(-255, 255),
        ],
        Kind::Outline => vec![c(a.color, 0x000000FF), b(a.inside), b(a.eight_connected)],
        Kind::DropShadow => vec![a.dx.unwrap_or(1), a.dy.unwrap_or(1), c(a.color, 0x00000080)],
        Kind::RemoveIsolatedPixels => vec![b(a.eight_connected)],
        Kind::RemoveBackground => {
            let flags = b(a.shadow) | (b(a.global) << 1);
            vec![
                a.tolerance.unwrap_or(28).clamp(0, 255),
                flags,
                a.shadow_depth.unwrap_or(55).clamp(1, 95),
                // Alpha 0 means "detect the border color".
                a.color.map_or(0, |c| (c.0 | 0xFF) as i32),
            ]
        }
        Kind::CleanEdges => {
            let halo =
                a.halo.is_some() || a.halo_rings.is_some() || a.halo_gain.is_some() || a.halo_saturation.is_some();
            let thin = a.thin_outline.is_some() || a.thickness.is_some();
            let named = halo || thin || a.despeckle.is_some() || a.holes.is_some() || a.light_specks.is_some();
            let on = |v: Option<bool>, other: bool| v.unwrap_or(other);
            let flags = if !named {
                7
            } else {
                (on(a.halo, halo) as i32)
                    | ((a.despeckle.is_some() as i32) << 1)
                    | ((on(a.thin_outline, thin) as i32) << 2)
                    | ((a.light_specks.unwrap_or(false) as i32) << 3)
            };
            let fixed = a.outline_color.is_some();
            vec![
                flags,
                a.halo_rings.unwrap_or(2).min(8) as i32,
                a.halo_gain.unwrap_or(40).min(255) as i32,
                a.halo_saturation.unwrap_or(30).min(100) as i32,
                a.despeckle.unwrap_or(4).min(1024) as i32,
                a.holes.unwrap_or(0).min(1024) as i32,
                a.thickness.unwrap_or(3).min(16) as i32,
                a.outline_luma.unwrap_or(70).min(255) as i32,
                fixed as i32,
                a.outline_color.map_or(0, |c| c.0 as i32),
                a.outline_tolerance.unwrap_or(0).min(255) as i32,
            ]
        }
        Kind::Curves => {
            let mut v = Vec::new();
            for curve in [&a.curve_rgb, &a.curve_red, &a.curve_green, &a.curve_blue, &a.curve_alpha] {
                let pts: Vec<[i32; 2]> = curve.clone().unwrap_or_else(|| vec![[0, 0], [255, 255]]);
                if pts.len() < 2 || pts.len() > 64 {
                    return Err("each curve needs 2-64 points".into());
                }
                v.push(pts.len() as i32);
                v.extend(pts.iter().flat_map(|p| [p[0].clamp(0, 255), p[1].clamp(0, 255)]));
            }
            v
        }
    })
}

fn filter(e: &mut Engine, a: FilterArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let target = a.target.unwrap_or(Target::Cel);
    let layer = match target {
        Target::Frame => d.layer(&a.layer)?,
        _ => d.image_layer(&a.layer)?,
    };
    let frame = d.frame(a.frame)?;
    let p = params(&a)?;
    let (x, y, w, h) = match a.region {
        Some(r) => {
            let (x, y, w, h) = clip(Some(r), d.doc.width(), d.doc.height())?;
            (x as i32, y as i32, w as u32, h as u32)
        }
        None => {
            let mut b = [0i32; 4];
            if unsafe { sel::pixmith_sel_bounds(raw, b.as_mut_ptr()) } {
                (b[0], b[1], b[2].max(0) as u32, b[3].max(0) as u32)
            } else {
                (0, 0, 0, 0)
            }
        }
    };
    let kind = a.kind as u32;
    let title = super::act(&a.kind);
    if a.preview.unwrap_or(false) {
        let (cw, ch) = (d.doc.width() as usize, d.doc.height() as usize);
        let mut out = vec![0u8; cw * ch * 4];
        clear_error();
        if !unsafe {
            filters::pixmith_filter_preview(
                raw,
                kind,
                target as u32,
                layer,
                frame,
                x,
                y,
                w,
                h,
                p.as_ptr(),
                p.len(),
                out.as_mut_ptr(),
                out.len(),
            )
        } {
            return Err(fail("could not preview the filter"));
        }
        // A region crops the preview (before scaling); a selection alone does not.
        let full = Img { w: cw, h: ch, px: out };
        let (full, crop) = match a.region {
            Some(_) => (full.crop(x as usize, y as usize, w as usize, h as usize), Some((x, y, w, h))),
            None => (full, None),
        };
        let scale = auto_scale(full.w, full.h, a.scale);
        let img = full.upscale(scale).on_background(parse_background(&a.background)?, (scale * 4).clamp(4, 16));
        let mut value = json!({ "doc": d.id, "preview": title, "frame": frame, "scale": scale, "applied": false });
        if let Some((x, y, w, h)) = crop {
            value["region"] = json!({ "x": x, "y": y, "w": w, "h": h });
            value["image_size"] = json!([img.w, img.h]);
            value["note"] = json!(format!(
                "the preview is cropped to the region {w}x{h} at ({x}, {y}) of the {cw}x{ch} canvas; nothing was changed"
            ));
        }
        return Ok(Out { value, images: vec![img.png()?], text: None });
    }
    // Remove Background reports what it found (the whole cel; a selection or region may keep part of it).
    let background = if a.kind == Kind::RemoveBackground && target == Target::Cel {
        let (cw, ch) = (d.doc.width() as usize, d.doc.height() as usize);
        let mut px = super::view::pixels(d, Some(layer), frame)?;
        BgOptions::from_params(&p).map(|o| {
            let st = background::remove_background(&mut px, cw, ch, &o);
            json!({
                "color": st.color.map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])),
                "background": st.background, "global": st.global, "shadow": st.shadow,
                "shadow_skipped": st.shadow_skipped, "removed": st.removed(),
            })
        })
    } else {
        None
    };
    let changed = d.edit_changed(&format!("Filter: {title}"), |_| {
        Ok(unsafe {
            filters::pixmith_filter_apply(raw, kind, target as u32, layer, frame, x, y, w, h, p.as_ptr(), p.len())
        })
    })?;
    Out::json(json!({
        "doc": d.id, "filter": title, "layer": layer, "frame": frame,
        "target": match target { Target::Cel => "cel", Target::Layer => "layer", Target::Frame => "frame" },
        "region": if w == 0 { json!("whole canvas") } else { json!({ "x": x, "y": y, "w": w, "h": h }) },
        "changed": changed,
        "background": background,
    }))
}
