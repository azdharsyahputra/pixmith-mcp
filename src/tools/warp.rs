//! Edit > Warp…: the pixel-art puppet / mesh warp of a cel.

use super::view::{auto_scale, parse_background};
use super::{CHANGE, Out, ToolDef, tool};
use crate::doc::{clear_error, fail};
use crate::engine::{Engine, R, Ref};
use crate::image::Img;
use pixmith_core::paint::warp as wp;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Transform",
        "warp",
        "Pixel-art warp",
        CHANGE,
        WarpArgs,
        warp,
        "Edit > Warp…: bend, stretch or nudge part of a cel like a puppet, without blur and without new colors. A `grid` of [cols, rows] handles (each 2-8, default [2, 2]) is spread over the selection bounds (the whole canvas when nothing is selected); `offsets` has one [dx, dy] per handle, row by row from the top-left (cols * rows entries; the content at that handle moves by that many pixels), interpolated bilinearly or, with smooth=true, with a Catmull-Rom spline. `pins` ([{x, y, dx, dy, radius}], up to 32) move the content within `radius` of a canvas point by up to (dx, dy), fading out smoothly. The result is resampled by majority vote (`samples` points per output pixel along each axis, 1-8, default 4): nothing is blended. Only selected pixels are read or changed. One undo step; returns changed=false when no pixel moved. The result lists the handle positions (`handles`, canvas coordinates) so you can aim the offsets. preview=true returns an image of the frame with the warp applied and changes nothing. Very thin 1 px diagonals can break under a strong bend."
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
pub struct PinArg {
    /// Canvas x of the pin (pixel corners are integers).
    pub x: f64,
    /// Canvas y of the pin.
    pub y: f64,
    /// How far the content at the pin moves along x.
    pub dx: f64,
    /// How far the content at the pin moves along y.
    pub dy: f64,
    /// Content within this many pixels is affected, fading out towards the edge.
    pub radius: f64,
}

#[derive(Deserialize, JsonSchema)]
pub struct WarpArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer (default: active).
    pub layer: Option<Ref>,
    /// Frame (default: active).
    pub frame: Option<u32>,
    /// Handle grid [cols, rows], each 2-8 (default [2, 2]).
    pub grid: Option<[u32; 2]>,
    /// One [dx, dy] per handle, row by row from the top-left (cols * rows entries; default all zeros).
    pub offsets: Option<Vec<[f64; 2]>>,
    /// Sample points per output pixel along each axis, 1-8 (default 4).
    pub samples: Option<u32>,
    /// Interpolate between handles with a Catmull-Rom spline instead of bilinearly.
    pub smooth: Option<bool>,
    /// Point handles (at most 32).
    pub pins: Option<Vec<PinArg>>,
    /// Return an image of the warped frame and change nothing.
    pub preview: Option<bool>,
    /// preview: integer upscale (default auto).
    pub scale: Option<u32>,
    /// preview: "checker" (default), "transparent" or a color.
    pub background: Option<String>,
}

fn warp(e: &mut Engine, a: WarpArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let raw = d.doc.raw();
    let layer = d.image_layer(&a.layer)?;
    let frame = d.frame(a.frame)?;
    let [cols, rows] = a.grid.unwrap_or([2, 2]);
    if !(2..=wp::MAX_GRID).contains(&cols) || !(2..=wp::MAX_GRID).contains(&rows) {
        return Err(format!("grid must be [cols, rows] with each 2-{}", wp::MAX_GRID));
    }
    let n = (cols * rows) as usize;
    let offsets: Vec<[f64; 2]> = match &a.offsets {
        Some(o) if o.len() != n => {
            return Err(format!(
                "grid [{cols}, {rows}] needs {n} offsets ([dx, dy] per handle, row by row), got {}",
                o.len()
            ));
        }
        Some(o) => o.clone(),
        None => vec![[0.0, 0.0]; n],
    };
    let flat: Vec<f64> = offsets.iter().flatten().copied().collect();
    let pins = a.pins.clone().unwrap_or_default();
    if pins.len() > wp::MAX_PINS {
        return Err(format!("at most {} pins", wp::MAX_PINS));
    }
    let pin_flat: Vec<f64> = pins.iter().flat_map(|p| [p.x, p.y, p.dx, p.dy, p.radius]).collect();
    if flat.iter().chain(&pin_flat).any(|v| !v.is_finite()) {
        return Err("offsets and pins must be finite numbers".into());
    }
    let samples = a.samples.unwrap_or(4).clamp(1, wp::MAX_SAMPLES);
    let smooth = a.smooth.unwrap_or(false);
    let mut region = [0i32; 4];
    if !unsafe { wp::pixmith_warp_bounds(raw, region.as_mut_ptr()) } {
        return Err(fail("could not read the warp region"));
    }
    let handles: Vec<Vec<[f64; 2]>> = (0..rows)
        .map(|j| {
            (0..cols)
                .map(|i| {
                    [
                        region[0] as f64 + region[2] as f64 * i as f64 / (cols - 1) as f64,
                        region[1] as f64 + region[3] as f64 * j as f64 / (rows - 1) as f64,
                    ]
                })
                .collect()
        })
        .collect();
    let region_json = json!({ "x": region[0], "y": region[1], "w": region[2], "h": region[3] });
    if a.preview.unwrap_or(false) {
        let (cw, ch) = (d.doc.width() as usize, d.doc.height() as usize);
        let mut out = vec![0u8; cw * ch * 4];
        clear_error();
        if !unsafe {
            wp::pixmith_warp_preview(
                raw,
                layer,
                frame,
                cols,
                rows,
                flat.as_ptr(),
                samples,
                smooth,
                pin_flat.as_ptr(),
                pins.len(),
                out.as_mut_ptr(),
                out.len(),
            )
        } {
            return Err(fail("could not preview the warp"));
        }
        let scale = auto_scale(cw, ch, a.scale);
        let img = Img { w: cw, h: ch, px: out }
            .upscale(scale)
            .on_background(parse_background(&a.background)?, (scale * 4).clamp(4, 16));
        return Ok(Out {
            value: json!({ "doc": d.id, "layer": layer, "frame": frame, "applied": false, "region": region_json, "handles": handles, "scale": scale }),
            images: vec![img.png()?],
            text: None,
        });
    }
    let changed = d.edit_changed("Warp", |_| {
        clear_error();
        Ok(unsafe {
            wp::pixmith_warp_apply(
                raw,
                layer,
                frame,
                cols,
                rows,
                flat.as_ptr(),
                samples,
                smooth,
                pin_flat.as_ptr(),
                pins.len(),
            )
        })
    })?;
    Out::json(json!({
        "doc": d.id, "layer": layer, "frame": frame, "grid": [cols, rows], "samples": samples, "smooth": smooth,
        "pins": pins.len(), "region": region_json, "handles": handles, "changed": changed,
    }))
}
