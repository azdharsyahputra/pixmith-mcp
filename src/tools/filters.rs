//! FX filters, applied as one undo step (or previewed as an image without changing anything).

use super::view::{Rect, auto_scale, clip, parse_background};
use super::{CHANGE, Out, ToolDef, tool};
use crate::color::Color;
use crate::doc::{clear_error, fail};
use crate::engine::{Engine, R, Ref};
use crate::image::Img;
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
        "Apply an FX filter to a cel (default), every frame of a layer (target=layer) or every layer of a frame (target=frame), limited to the selection's bounds when something is selected or to `region`. Kinds and their parameters: replace_color (from_color, to_color, tolerance), invert, desaturate, hsl (hue -180..180, saturation and lightness -100..100), brightness_contrast (brightness, contrast -100..100), color_balance (red, green, blue offsets -255..255), outline (color, inside, eight_connected), drop_shadow (dx, dy, color), remove_isolated_pixels (eight_connected), blur, sharpen (3x3, alpha-aware), curves (curve_rgb / curve_red / curve_green / curve_blue / curve_alpha: [[x, y], ...] control points in 0-255). preview=true returns an image of the result and changes nothing."
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
    /// Return a preview image instead of applying.
    pub preview: Option<bool>,
    /// replace_color: color to replace.
    pub from_color: Option<Color>,
    /// replace_color: replacement.
    pub to_color: Option<Color>,
    /// replace_color: 0-255.
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
    /// outline/drop_shadow: color (default black).
    pub color: Option<Color>,
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
        let scale = auto_scale(cw, ch, a.scale);
        let img = Img { w: cw, h: ch, px: out }
            .upscale(scale)
            .on_background(parse_background(&a.background)?, (scale * 4).clamp(4, 16));
        return Ok(Out {
            value: json!({ "doc": d.id, "preview": title, "frame": frame, "scale": scale, "applied": false }),
            images: vec![img.png()?],
            text: None,
        });
    }
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
    }))
}
