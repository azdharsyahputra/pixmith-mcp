//! Looking at a document: `render` returns PNG images the agent can see, `get_pixels` / `get_pixel` exact
//! values.

use super::{Out, READ, ToolDef, tool};
use crate::color::{Color, hex};
use crate::engine::{DocEntry, Engine, R, Ref};
use crate::image::{Background, Img};
use pixmith_core::document::Sprite;
use pixmith_core::document::layers::render_frame_rect;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Viewing",
            "render",
            "Render to image",
            READ,
            RenderArgs,
            render,
            "Render the sprite as a PNG image you can look at (MCP image content). Defaults: the active frame, all visible layers composited, scaled up so the long side is about 512 px, on a light checkerboard so transparency is visible. Options: other frames (up to 16 images at once), a single layer or group in isolation (shown even if hidden), a region, an explicit integer scale, a solid/transparent background, and onion skin (neighboring frames tinted red = before, blue = after). Never changes the document. For exact colors use get_pixels."
        ),
        tool!(
            "Viewing",
            "get_pixels",
            "Read pixels",
            READ,
            PixelsArgs,
            get_pixels,
            "Read exact pixel colors of a rectangle, from the composited image or from one layer's cel (raw cel pixels, before layer/cel opacity). The default `legend` format gives one string per row with one character per pixel plus a legend mapping characters to #rrggbbaa colors (and palette indices) — compact and easy to reason about; `hex` gives a 2-D array of colors. At most 16384 pixels per call (4096 for hex); page through larger areas."
        ),
        tool!(
            "Viewing",
            "get_pixel",
            "Read one pixel",
            READ,
            PixelArgs,
            get_pixel,
            "Read the color at one pixel: the composited color plus the value of every layer that has a non-transparent pixel there (top first), and the palette index if the color is in the palette."
        ),
    ]
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    /// Width in pixels.
    pub w: u32,
    /// Height in pixels.
    pub h: u32,
}

#[derive(Deserialize, JsonSchema)]
pub struct Onion {
    /// Frames before the current one to show (tinted red). Default 1.
    pub before: Option<u32>,
    /// Frames after the current one to show (tinted blue). Default 1.
    pub after: Option<u32>,
    /// Opacity of the onion frames, 0-1. Default 0.35.
    pub opacity: Option<f32>,
}

#[derive(Deserialize, JsonSchema)]
pub struct RenderArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Frame to render (default: the active frame).
    pub frame: Option<u32>,
    /// Several frames, one image each (max 16). Overrides `frame`.
    pub frames: Option<Vec<u32>>,
    /// Render only this layer (index or name; a group renders with its children), even if hidden.
    pub layer: Option<Ref>,
    /// Only this rectangle of the canvas.
    pub region: Option<Rect>,
    /// Integer upscale 1-64 (default: auto, long side about 512 px; output is capped at 4096 px).
    pub scale: Option<u32>,
    /// "checker" (default), "transparent", or a color such as "#ffffff".
    pub background: Option<String>,
    /// Show neighboring frames faintly (off by default).
    pub onion_skin: Option<Onion>,
}

/// The sprite with only layer `i` shown (plus the groups around it, and its children if it is a group).
fn isolate(s: &Sprite, i: usize) -> Sprite {
    let mut v = s.clone();
    let n = v.layers.len();
    let mut keep = vec![false; n];
    keep[i] = true;
    let mut level = v.layers[i].child_level;
    for j in (0..i).rev() {
        if level == 0 {
            break;
        }
        if v.layers[j].child_level < level {
            level = v.layers[j].child_level;
            keep[j] = true;
        }
    }
    let own = v.layers[i].child_level;
    // A group's children follow it in the list, one level deeper.
    for (k, l) in keep.iter_mut().zip(&v.layers).skip(i + 1) {
        if l.child_level <= own {
            break;
        }
        *k = l.visible;
    }
    for (j, (l, k)) in v.layers.iter_mut().zip(keep).enumerate() {
        l.visible = k || (j == i);
    }
    v
}

pub fn parse_background(s: &Option<String>) -> R<Background> {
    Ok(match s.as_deref().map(str::trim) {
        None | Some("checker") | Some("checkerboard") => Background::Checker,
        Some("transparent") | Some("none") => Background::Transparent,
        Some(c) => Background::Color(Color::parse(c)?.0),
    })
}

/// Clips a rectangle to the canvas; `None` means the whole canvas.
pub fn clip(region: Option<Rect>, w: u32, h: u32) -> R<(usize, usize, usize, usize)> {
    let Some(r) = region else { return Ok((0, 0, w as usize, h as usize)) };
    let x0 = r.x.max(0) as i64;
    let y0 = r.y.max(0) as i64;
    let x1 = (r.x as i64 + r.w as i64).min(w as i64);
    let y1 = (r.y as i64 + r.h as i64).min(h as i64);
    if x1 <= x0 || y1 <= y0 {
        return Err(format!("the region {}x{} at ({}, {}) is outside the {w}x{h} canvas", r.w, r.h, r.x, r.y));
    }
    Ok((x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize))
}

pub fn auto_scale(w: usize, h: usize, scale: Option<u32>) -> usize {
    let long = w.max(h).max(1);
    let cap = (4096 / long).max(1);
    match scale {
        Some(s) if s > 0 => (s as usize).min(64).min(cap),
        _ => (512 / long).clamp(1, 32).min(cap),
    }
}

fn render(e: &mut Engine, a: RenderArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let frames = match &a.frames {
        Some(f) if f.is_empty() => return Err("frames is empty".into()),
        Some(f) if f.len() > 16 => return Err("at most 16 frames per call".into()),
        Some(f) => f.iter().map(|&x| d.frame(Some(x))).collect::<R<Vec<_>>>()?,
        None => vec![d.frame(a.frame)?],
    };
    let bg = parse_background(&a.background)?;
    let (w, h) = (d.doc.width(), d.doc.height());
    let rect = clip(a.region, w, h)?;
    let scale = auto_scale(rect.2, rect.3, a.scale);
    let base = d.doc.sprite();
    let isolated = match &a.layer {
        Some(_) => Some(isolate(base, d.layer(&a.layer)? as usize)),
        None => None,
    };
    let s = isolated.as_ref().unwrap_or(base);
    let n = d.doc.frame_count();
    let draw = |f: u32| Img { w: rect.2, h: rect.3, px: render_frame_rect(s, f as usize, rect) };
    let mut images = Vec::new();
    for &f in &frames {
        let mut img = Img::new(rect.2, rect.3);
        if let Some(o) = &a.onion_skin {
            let op = o.opacity.unwrap_or(0.35).clamp(0.0, 1.0);
            for k in (1..=o.before.unwrap_or(1).min(8)).rev() {
                if f >= k {
                    img.draw_over(&draw(f - k).tinted([255, 64, 64]), op / k as f32);
                }
            }
            for k in (1..=o.after.unwrap_or(1).min(8)).rev() {
                if f + k < n {
                    img.draw_over(&draw(f + k).tinted([64, 96, 255]), op / k as f32);
                }
            }
        }
        img.draw_over(&draw(f), 1.0);
        let out = img.upscale(scale).on_background(bg, (scale * 4).clamp(4, 16));
        images.push(out.png()?);
    }
    let layer = a.layer.as_ref().map(|_| d.layer(&a.layer)).transpose()?;
    Ok(Out {
        value: json!({
            "doc": d.id, "frames": frames, "layer": layer,
            "region": { "x": rect.0, "y": rect.1, "w": rect.2, "h": rect.3 },
            "scale": scale, "image_size": [rect.2 * scale, rect.3 * scale],
            "note": "each canvas pixel is scale x scale image pixels",
        }),
        images,
        text: None,
    })
}

/// Pixels of one layer's cel at `frame` (canvas sized), or the composite when `layer` is `None`.
pub fn pixels(d: &DocEntry, layer: Option<u32>, frame: u32) -> R<Vec<u8>> {
    match layer {
        None => d.doc.render(frame),
        Some(l) => {
            let s = d.doc.sprite();
            if s.layers[l as usize].is_group() {
                return Ok(isolate(s, l as usize).render(frame as usize));
            }
            let len = s.width as usize * s.height as usize * 4;
            Ok(s.cels
                .get(frame as usize)
                .and_then(|row| row.get(l as usize))
                .and_then(|c| c.as_ref())
                .map(|c| c.pixels.clone())
                .unwrap_or_else(|| vec![0; len]))
        }
    }
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Legend,
    Hex,
}

#[derive(Deserialize, JsonSchema)]
pub struct PixelsArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Read one layer's cel (index or name). Omit for the composited image of all visible layers.
    pub layer: Option<Ref>,
    /// Frame (default: the active frame).
    pub frame: Option<u32>,
    /// Rectangle to read (default: the whole canvas, if small enough).
    pub region: Option<Rect>,
    /// legend (default) or hex.
    pub format: Option<PixelFormat>,
}

const LEGEND_CHARS: &str = ".abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789#$%&*+-=?@^~!:;<>/|()[]{}_";

fn get_pixels(e: &mut Engine, a: PixelsArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let frame = d.frame(a.frame)?;
    let layer = a.layer.as_ref().map(|_| d.layer(&a.layer)).transpose()?;
    let (w, h) = (d.doc.width(), d.doc.height());
    let (x0, y0, rw, rh) = clip(a.region, w, h)?;
    let format = a.format.unwrap_or(PixelFormat::Legend);
    let limit = if format == PixelFormat::Hex { 4096 } else { 16384 };
    if rw * rh > limit {
        return Err(format!(
            "{rw}x{rh} = {} pixels is more than {limit} per call; pass a smaller region (e.g. {{\"x\":{x0},\"y\":{y0},\"w\":{},\"h\":{}}}) and page through",
            rw * rh,
            rw.min(128),
            (limit / rw.min(128)).min(rh)
        ));
    }
    let px = pixels(d, layer, frame)?;
    let at = |x: usize, y: usize| {
        let i = (y * w as usize + x) * 4;
        u32::from_be_bytes([px[i], px[i + 1], px[i + 2], px[i + 3]])
    };
    let pal = super::docs::palette(d);
    let pal_index = |c: u32| pal.iter().position(|&p| p == c);
    let base = json!({ "doc": d.id, "frame": frame, "layer": layer, "x": x0, "y": y0, "w": rw, "h": rh });
    let mut out = base.as_object().cloned().expect("object");
    let chars: Vec<char> = LEGEND_CHARS.chars().collect();
    let mut legend: BTreeMap<u32, char> = BTreeMap::new();
    let mut overflow = false;
    if format == PixelFormat::Legend {
        // Transparent is always '.', then colors in order of appearance.
        let mut next = 1;
        'scan: for y in y0..y0 + rh {
            for x in x0..x0 + rw {
                let c = at(x, y);
                let c = if c & 0xFF == 0 { 0 } else { c };
                if c == 0 || legend.contains_key(&c) {
                    continue;
                }
                if next >= chars.len() {
                    overflow = true;
                    break 'scan;
                }
                legend.insert(c, chars[next]);
                next += 1;
            }
        }
    }
    if format == PixelFormat::Hex || overflow {
        if rw * rh > 4096 {
            return Err(format!(
                "the region has more than {} distinct colors, which needs the hex format, limited to 4096 pixels per call; read a smaller region",
                chars.len() - 1
            ));
        }
        let rows: Vec<Vec<String>> = (y0..y0 + rh).map(|y| (x0..x0 + rw).map(|x| hex(at(x, y))).collect()).collect();
        out.insert("format".into(), "hex".into());
        out.insert("rows".into(), json!(rows));
    } else {
        let rows: Vec<String> = (y0..y0 + rh)
            .map(|y| {
                (x0..x0 + rw)
                    .map(|x| {
                        let c = at(x, y);
                        if c & 0xFF == 0 { '.' } else { legend[&c] }
                    })
                    .collect()
            })
            .collect();
        let mut lg = serde_json::Map::new();
        lg.insert(".".into(), json!("transparent"));
        let mut entries: Vec<(char, u32)> = legend.iter().map(|(&c, &ch)| (ch, c)).collect();
        entries.sort_by_key(|(ch, _)| LEGEND_CHARS.find(*ch));
        for (ch, c) in entries {
            let v = match pal_index(c) {
                Some(i) => json!(format!("{} (palette {i})", hex(c))),
                None => json!(hex(c)),
            };
            lg.insert(ch.to_string(), v);
        }
        out.insert("format".into(), "legend".into());
        out.insert("legend".into(), Value::Object(lg));
        out.insert("rows".into(), json!(rows));
    }
    Out::json(Value::Object(out))
}

#[derive(Deserialize, JsonSchema)]
pub struct PixelArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    pub x: i32,
    pub y: i32,
    /// Frame (default: the active frame).
    pub frame: Option<u32>,
}

fn get_pixel(e: &mut Engine, a: PixelArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let frame = d.frame(a.frame)?;
    let (w, h) = (d.doc.width() as i32, d.doc.height() as i32);
    if a.x < 0 || a.y < 0 || a.x >= w || a.y >= h {
        return Err(format!("({}, {}) is outside the {w}x{h} canvas", a.x, a.y));
    }
    let i = (a.y as usize * w as usize + a.x as usize) * 4;
    let read = |p: &[u8]| u32::from_be_bytes([p[i], p[i + 1], p[i + 2], p[i + 3]]);
    let composite = read(&d.doc.render(frame)?);
    let pal = super::docs::palette(d);
    let mut layers = Vec::new();
    for l in (0..d.layer_count()).rev() {
        if d.is_group(l) {
            continue;
        }
        let c = read(&pixels(d, Some(l), frame)?);
        if c & 0xFF != 0 {
            layers.push(json!({ "layer": l, "name": d.layer_name(l), "color": hex(c) }));
        }
    }
    Out::json(json!({
        "doc": d.id, "x": a.x, "y": a.y, "frame": frame,
        "color": hex(composite),
        "palette_index": pal.iter().position(|&p| p == composite),
        "layers_top_first": layers,
    }))
}
