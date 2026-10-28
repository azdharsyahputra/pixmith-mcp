//! Drawing tools. Every call is one undo step and goes through the core's painter, so ink modes, brush size
//! and shape, custom brushes, mirroring and the selection (drawing is clipped to it) apply like in the apps.

use super::{ADD, CHANGE, Hints, Out, ToolDef, tool};
use crate::color::Color;
use crate::doc::cstr;
use crate::engine::{DocEntry, Engine, R, Ref, Undo};
use pixmith_core::Cel;
use pixmith_core::paint::{curves, font, ink, retouch, shapes};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "Drawing",
            "draw_pixels",
            "Set pixels",
            ADD,
            PixelsArgs,
            draw_pixels,
            "Pencil for exact pixels: paint individual pixels (batch, up to 65536); painting color \"transparent\" is the Eraser. Either give `pixels` as [{x, y, color}] or `points` as [[x, y], ...] with one `color`. Uses the brush and ink options (default: 1 px square brush, simple ink = replace the pixel; a transparent color erases)."
        ),
        tool!(
            "Drawing",
            "draw_shape",
            "Line / rectangle / ellipse",
            ADD,
            ShapeArgs,
            draw_shape,
            "The Line, Rectangle and Ellipse tools: draw a line, rectangle or ellipse between two points (`from`, `to`; for rectangles and ellipses they are opposite corners of the bounding box, inclusive). `filled` fills rectangles and ellipses. Brush, ink and mirror options apply."
        ),
        tool!(
            "Drawing",
            "draw_polygon",
            "Polygon",
            ADD,
            PolygonArgs,
            draw_polygon,
            "The Polygon tool, and the Contour tool (freehand outline: pass the traced points with filled=true): draw a polygon through `points` ([[x, y], ...], at least 2). Closed by default; `filled` fills it; closed=false draws an open polyline."
        ),
        tool!(
            "Drawing",
            "draw_curve",
            "Bezier curve",
            ADD,
            CurveArgs,
            draw_curve,
            "The Curve tool: draw a cubic Bezier curve: `points` = [start, control1, control2, end], each [x, y]."
        ),
        tool!(
            "Drawing",
            "draw_stroke",
            "Freehand stroke",
            ADD,
            StrokeArgs,
            draw_stroke,
            "The Pencil and Eraser tools: draw a freehand stroke along a path of points ([[x, y], ...]); consecutive points are joined. To use the Eraser, pass color \"transparent\". pixel_perfect is the Pixel-perfect Pencil option (removes the doubled corner pixels of diagonal strokes). Also: brush_size, brush_shape circle (Round Brush), a captured custom brush (brush tool), ink (Simple, Alpha, Lock Alpha, Behind, Shading) and mirror (symmetry horizontal / vertical / both)."
        ),
        tool!(
            "Drawing",
            "retouch",
            "Blur / Jumble brush",
            CHANGE,
            RetouchArgs,
            retouch,
            "The app's Blur and Jumble brushes: rework the pixels under a brush stroke along `points` instead of painting a color. kind blur smooths each pixel with its 3x3 neighbours (alpha-aware), mixed by `strength` 1-100; jumble swaps pixels with random ones nearby (about size/2 away), each with probability `strength`%, repeatable with `seed`. `size` is the brush size 1-64; brush_shape circle, a captured custom brush and mirror apply; ink and colors do not. Clipped to the selection; one undo step per call."
        ),
        tool!(
            "Drawing",
            "fill",
            "Flood fill",
            CHANGE,
            FillArgs,
            fill,
            "The Paint Bucket tool: fill at (x, y). contiguous=true (default) fills the connected area of the clicked color; false replaces that color everywhere in the cel. tolerance 0-255 also takes colors whose channels differ by at most that much. Optional dither `pattern` between `color` and `color2`. Fills only inside the selection when there is one."
        ),
        tool!(
            "Drawing",
            "spray",
            "Spray paint",
            ADD,
            SprayArgs,
            spray,
            "The Spray tool: spray-paint random pixels in a circle of `radius` around each point; `density` 1-100 controls how many pixels per puff, `seed` makes it repeatable."
        ),
        tool!(
            "Drawing",
            "gradient",
            "Gradient fill",
            CHANGE,
            GradientArgs,
            gradient,
            "The Gradient tool: fill the selection (or the whole cel) with a linear or radial gradient from `from` to `to`, `color_from` to `color_to`; dither 2/4/8 uses an ordered Bayer pattern instead of smooth blending (pixel-art style)."
        ),
        tool!(
            "Drawing",
            "draw_text",
            "Pixel text",
            ADD,
            TextArgs,
            draw_text,
            "The Text tool: write text with the built-in 5x8 pixel font, top-left at (x, y); each font pixel becomes scale x scale pixels. Returns the text's size. With measure_only=true nothing is drawn."
        ),
        tool!(
            "Drawing",
            "brush",
            "Custom brush",
            BRUSH,
            BrushArgs,
            brush,
            "Custom brush (Brush from Selection / Reset Brush): `capture` turns the selected pixels of a layer into the brush used by every later drawing call (until `reset`); `info` reports it. The brush is shared by all documents."
        ),
    ]
}

/// Changes the server-wide brush, not documents.
const BRUSH: Hints = Hints { read_only: false, destructive: false, idempotent: true, files: false };

#[derive(Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum InkMode {
    #[default]
    Simple,
    Alpha,
    LockAlpha,
    Behind,
    Shading,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BrushShape {
    #[default]
    Square,
    Circle,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, Default, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Mirror {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

impl Mirror {
    pub fn flags(self) -> u32 {
        match self {
            Mirror::None => 0,
            Mirror::Horizontal => 1,
            Mirror::Vertical => 2,
            Mirror::Both => 3,
        }
    }
}

/// Options shared by the drawing tools.
#[derive(Deserialize, JsonSchema, Default)]
pub struct Paint {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Layer index or name (default: the active layer). Must not be a group.
    pub layer: Option<Ref>,
    /// Frame (default: the active frame).
    pub frame: Option<u32>,
    /// Brush size in pixels, 1-64 (default 1).
    pub brush_size: Option<u32>,
    /// square (default) or circle (round brush for sizes above 1).
    pub brush_shape: Option<BrushShape>,
    /// simple (default; replaces pixels, transparent erases), alpha (blend over), lock_alpha (recolor only opaque pixels), behind (paint under existing pixels), shading (step pixels along shading_ramp).
    pub ink: Option<InkMode>,
    /// Colors the shading ink steps through (default: the palette).
    pub shading_ramp: Option<Vec<Color>>,
    /// Shading steps toward the start of the ramp instead of the end.
    pub shading_reverse: Option<bool>,
    /// Symmetry: also draw mirrored (horizontal = left/right, vertical = top/bottom) across the canvas center, or
    /// across `mirror_axis_x` / `mirror_axis_y`.
    pub mirror: Option<Mirror>,
    /// Where the left/right mirror line is, in pixels from the left edge (default: the canvas center). 16 is the line
    /// between columns 15 and 16 (column 15 mirrors to 16); 16.5 runs through column 16 (it stays put and 15 mirrors
    /// to 17). Steps of 0.5.
    pub mirror_axis_x: Option<f64>,
    /// Where the top/bottom mirror line is, in pixels from the top edge (default: the canvas center); like
    /// `mirror_axis_x`.
    pub mirror_axis_y: Option<f64>,
}

impl Paint {
    pub fn size(&self) -> u32 {
        self.brush_size.unwrap_or(1).clamp(1, 64)
    }

    pub fn flags(&self) -> u32 {
        self.mirror.unwrap_or_default().flags()
    }

    /// The color to paint with `c` under this call's ink: the shading ink only steps existing pixels along its ramp,
    /// so a transparent color (which would make the brush erase) is replaced by an opaque one there.
    pub fn ink_color(&self, c: u32) -> u32 {
        if self.ink.unwrap_or_default() == InkMode::Shading && c & 0xFF == 0 { c | 0xFF } else { c }
    }

    /// Sets the core's (thread-wide) ink, brush shape and mirror axes for this call.
    pub fn apply(&self, d: &DocEntry) {
        let mode = self.ink.unwrap_or_default();
        let mut ramp: Vec<u32> = self.shading_ramp.iter().flatten().map(|c| c.0).collect();
        if mode == InkMode::Shading && ramp.is_empty() {
            ramp = super::docs::palette(d);
        }
        let p = if ramp.is_empty() { std::ptr::null() } else { ramp.as_ptr() };
        unsafe { ink::pixmith_ink_set(mode as u32, self.shading_reverse.unwrap_or(false), p, ramp.len()) };
        ink::pixmith_brush_circle(self.brush_shape.unwrap_or_default() == BrushShape::Circle);
        let doubled = |a: Option<f64>| a.map_or(-1, |v| (v * 2.0).round().max(0.0) as i32);
        shapes::pixmith_set_mirror_axes(doubled(self.mirror_axis_x), doubled(self.mirror_axis_y));
    }

    /// Checks the mirror axes against the canvas (`dims`): each must lie on it, in steps of 0.5.
    pub fn check_axes(&self, dims: (u32, u32)) -> R<()> {
        for (name, v, size) in
            [("mirror_axis_x", self.mirror_axis_x, dims.0), ("mirror_axis_y", self.mirror_axis_y, dims.1)]
        {
            let Some(v) = v else { continue };
            if !v.is_finite() || v < 0.5 || v > size as f64 - 0.5 {
                return Err(format!("{name} {v} is off the canvas: use 0.5 to {}", size as f64 - 0.5));
            }
            if (v * 2.0).fract() != 0.0 {
                return Err(format!("{name} {v} must be a whole or half pixel (e.g. 16 or 16.5)"));
            }
        }
        Ok(())
    }
}

fn cel(d: &DocEntry, layer: u32, frame: u32) -> Option<Arc<Cel>> {
    d.doc.sprite().cels.get(frame as usize)?.get(layer as usize)?.clone()
}

fn diff(a: &Option<Arc<Cel>>, b: &Option<Arc<Cel>>) -> usize {
    let empty = |c: &Option<Arc<Cel>>| c.as_ref().is_none_or(|c| c.pixels.chunks_exact(4).all(|p| p[3] == 0));
    match (a, b) {
        (Some(x), Some(y)) if Arc::ptr_eq(x, y) => 0,
        (Some(x), Some(y)) if x.pixels.len() == y.pixels.len() => {
            x.pixels.chunks_exact(4).zip(y.pixels.chunks_exact(4)).filter(|(p, q)| p != q).count()
        }
        (Some(x), None) | (None, Some(x)) => {
            if empty(&Some(x.clone())) {
                0
            } else {
                x.pixels.chunks_exact(4).filter(|p| p[3] != 0).count()
            }
        }
        (None, None) => 0,
        _ => 1,
    }
}

/// Runs a drawing operation on (layer, frame) as one undo step and reports how many pixels of that cel
/// changed. A drawing that changed nothing leaves no undo step.
pub fn paint_op(e: &mut Engine, p: &Paint, label: &str, f: impl FnOnce(&DocEntry, u32, u32)) -> R<Out> {
    let d = e.doc(&p.doc)?;
    let layer = d.image_layer(&p.layer)?;
    let frame = d.frame(p.frame)?;
    let sprite = d.doc.sprite();
    p.check_axes((sprite.width, sprite.height))?;
    let before = cel(d, layer, frame);
    let (undo0, _) = d.doc.history_counts();
    d.edit(label, Undo::Step, |d| {
        p.apply(d);
        f(d, layer, frame);
        Ok(())
    })?;
    let after = cel(d, layer, frame);
    let changed = diff(&before, &after);
    if changed == 0 && d.doc.history_counts().0 > undo0 && d.txns.is_empty() {
        d.doc.rollback();
    }
    Out::json(json!({
        "doc": d.id, "layer": layer, "frame": frame, "pixels_changed": changed,
        "note": if changed == 0 { Some("nothing changed: outside the canvas or selection, same color, or the ink left pixels as they were") } else { None },
    }))
}

#[derive(Deserialize, JsonSchema)]
pub struct PixelSpec {
    pub x: i32,
    pub y: i32,
    pub color: Color,
}

#[derive(Deserialize, JsonSchema)]
pub struct PixelsArgs {
    /// Pixels with individual colors.
    pub pixels: Option<Vec<PixelSpec>>,
    /// Pixel positions [[x, y], ...] painted with `color`.
    pub points: Option<Vec<[i32; 2]>>,
    /// Color for `points`.
    pub color: Option<Color>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_pixels(e: &mut Engine, a: PixelsArgs) -> R<Out> {
    let mut list: Vec<(i32, i32, u32)> = a.pixels.iter().flatten().map(|p| (p.x, p.y, p.color.0)).collect();
    if let Some(points) = &a.points {
        let c = a.color.ok_or("`points` needs `color`")?;
        list.extend(points.iter().map(|p| (p[0], p[1], c.0)));
    }
    if list.is_empty() {
        return Err("give `pixels` or `points`".into());
    }
    if list.len() > 65536 {
        return Err("at most 65536 pixels per call".into());
    }
    // A position listed twice is painted once, with its last color (the shading ink would otherwise step it twice).
    let mut seen = std::collections::HashSet::new();
    list.reverse();
    list.retain(|&(x, y, _)| seen.insert((x, y)));
    list.reverse();
    for px in &mut list {
        px.2 = a.paint.ink_color(px.2);
    }
    let (size, flags) = (a.paint.size(), a.paint.flags());
    paint_op(e, &a.paint, "Pencil", |d, l, f| {
        for &(x, y, c) in &list {
            unsafe { shapes::pixmith_shape(d.doc.raw(), 0, l, f, x, y, x, y, c, size, flags, false) };
        }
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ShapeKind {
    Line,
    Rectangle,
    Ellipse,
}

#[derive(Deserialize, JsonSchema)]
pub struct ShapeArgs {
    pub shape: ShapeKind,
    /// Start point / first corner [x, y].
    pub from: [i32; 2],
    /// End point / opposite corner [x, y] (inclusive).
    pub to: [i32; 2],
    pub color: Color,
    /// Fill rectangles and ellipses (default: outline only).
    pub filled: Option<bool>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_shape(e: &mut Engine, a: ShapeArgs) -> R<Out> {
    let kind = a.shape as u32;
    let flags = a.paint.flags() | if a.filled.unwrap_or(false) { 8 } else { 0 };
    let size = a.paint.size();
    let label = ["Line", "Rectangle", "Ellipse"][kind as usize];
    paint_op(e, &a.paint, label, |d, l, f| unsafe {
        shapes::pixmith_shape(
            d.doc.raw(),
            kind,
            l,
            f,
            a.from[0],
            a.from[1],
            a.to[0],
            a.to[1],
            a.paint.ink_color(a.color.0),
            size,
            flags,
            false,
        )
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct PolygonArgs {
    /// Vertices [[x, y], ...], at least 2, at most 4096.
    pub points: Vec<[i32; 2]>,
    pub color: Color,
    /// Fill the interior (closed polygons only).
    pub filled: Option<bool>,
    /// Join the last point back to the first (default true).
    pub closed: Option<bool>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_polygon(e: &mut Engine, a: PolygonArgs) -> R<Out> {
    if a.points.len() < 2 || a.points.len() > 4096 {
        return Err("a polygon needs 2-4096 points".into());
    }
    let closed = a.closed.unwrap_or(true);
    let flags = a.paint.flags() | if a.filled.unwrap_or(false) && closed { 8 } else { 0 } | if closed { 0 } else { 32 };
    let flat: Vec<i32> = a.points.iter().flat_map(|p| [p[0], p[1]]).collect();
    let size = a.paint.size();
    paint_op(e, &a.paint, "Polygon", |d, l, f| unsafe {
        curves::pixmith_polygon(
            d.doc.raw(),
            l,
            f,
            flat.as_ptr(),
            a.points.len(),
            a.paint.ink_color(a.color.0),
            size,
            flags,
            false,
        )
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct CurveArgs {
    /// Exactly 4 points: [start, control1, control2, end], each [x, y].
    pub points: Vec<[i32; 2]>,
    pub color: Color,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_curve(e: &mut Engine, a: CurveArgs) -> R<Out> {
    if a.points.len() != 4 {
        return Err("a Bezier curve needs exactly 4 points: start, control1, control2, end".into());
    }
    let flat: Vec<i32> = a.points.iter().flat_map(|p| [p[0], p[1]]).collect();
    let (size, flags) = (a.paint.size(), a.paint.flags());
    paint_op(e, &a.paint, "Curve", |d, l, f| unsafe {
        curves::pixmith_curve(d.doc.raw(), l, f, flat.as_ptr(), a.paint.ink_color(a.color.0), size, flags, false)
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct StrokeArgs {
    /// Path [[x, y], ...] (1-65536 points).
    pub points: Vec<[i32; 2]>,
    pub color: Color,
    /// Pixel-perfect pencil (removes L-shaped corner doubles).
    pub pixel_perfect: Option<bool>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_stroke(e: &mut Engine, a: StrokeArgs) -> R<Out> {
    if a.points.is_empty() || a.points.len() > 65536 {
        return Err("a stroke needs 1-65536 points".into());
    }
    let flags = a.paint.flags() | if a.pixel_perfect.unwrap_or(false) { 4 } else { 0 };
    let size = a.paint.size();
    paint_op(e, &a.paint, "Pencil", |d, l, f| {
        let raw = d.doc.raw();
        let p0 = a.points[0];
        unsafe { shapes::pixmith_stroke_begin(raw, l, f, p0[0], p0[1], a.paint.ink_color(a.color.0), size, flags) };
        for p in &a.points[1..] {
            unsafe { shapes::pixmith_stroke_to(raw, p[0], p[1]) };
        }
        shapes::pixmith_stroke_end_doc(raw);
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RetouchKind {
    Blur,
    Jumble,
}

#[derive(Deserialize, JsonSchema)]
pub struct RetouchArgs {
    /// blur or jumble.
    pub kind: RetouchKind,
    /// Path [[x, y], ...] (1-65536 points); consecutive points are joined by a line of dabs.
    pub points: Vec<[i32; 2]>,
    /// Brush size in pixels, 1-64 (default: brush_size, else 3).
    pub size: Option<u32>,
    /// Effect strength in percent, 1-100 (default 50). Blur: mix with the neighbourhood average. Jumble: chance each pixel is swapped.
    pub strength: Option<u32>,
    /// jumble: random seed (default 1). The same seed gives the same result.
    pub seed: Option<u32>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn retouch(e: &mut Engine, a: RetouchArgs) -> R<Out> {
    if a.points.is_empty() || a.points.len() > 65536 {
        return Err("a retouch stroke needs 1-65536 points".into());
    }
    let size = a.size.or(a.paint.brush_size).unwrap_or(3).clamp(1, 64);
    let strength = a.strength.unwrap_or(50).clamp(1, 100);
    let seed = a.seed.unwrap_or(1);
    let kind = match a.kind {
        RetouchKind::Blur => retouch::KIND_BLUR,
        RetouchKind::Jumble => retouch::KIND_JUMBLE,
    };
    let flags = a.paint.flags();
    let label = if a.kind == RetouchKind::Blur { "Blur" } else { "Jumble" };
    paint_op(e, &a.paint, label, |d, l, f| {
        let raw = d.doc.raw();
        let p0 = a.points[0];
        unsafe { retouch::pixmith_retouch_begin(raw, l, f, p0[0], p0[1], kind, size, strength, seed, flags) };
        for p in &a.points[1..] {
            unsafe { retouch::pixmith_retouch_to(raw, p[0], p[1]) };
        }
        retouch::pixmith_retouch_end_doc(raw);
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Pattern {
    Solid,
    Checker,
    Dither25,
    Dither50,
    Dither75,
}

#[derive(Deserialize, JsonSchema)]
pub struct FillArgs {
    pub x: i32,
    pub y: i32,
    pub color: Color,
    /// Connected area only (default true); false = every pixel of that color in the cel.
    pub contiguous: Option<bool>,
    /// 0-255: also fill colors whose R, G, B and A each differ by at most this (default 0 = exact).
    pub tolerance: Option<u32>,
    /// solid (default), checker (2x2 squares), dither25/dither50/dither75 (percent of `color`, the rest `color2`).
    pub pattern: Option<Pattern>,
    /// Second color for patterns (default transparent).
    pub color2: Option<Color>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn fill(e: &mut Engine, a: FillArgs) -> R<Out> {
    let contiguous = a.contiguous.unwrap_or(true);
    let tolerance = a.tolerance.unwrap_or(0).min(255);
    let pattern = a.pattern.unwrap_or(Pattern::Solid);
    let flags = a.paint.flags();
    if tolerance == 0 {
        let other = a.color2.map_or(0, |c| c.0);
        return paint_op(e, &a.paint, "Fill", |d, l, f| unsafe {
            ink::pixmith_fill_pattern(
                d.doc.raw(),
                l,
                f,
                a.x,
                a.y,
                a.paint.ink_color(a.color.0),
                other,
                pattern as u32,
                contiguous,
                flags,
            )
        });
    }
    if pattern != Pattern::Solid {
        return Err("patterns need tolerance 0".into());
    }
    // Same color test as the magic wand, painted through a coverage mask so ink and selection still apply.
    let d = e.doc(&a.paint.doc)?;
    let layer = d.image_layer(&a.paint.layer)?;
    let frame = d.frame(a.paint.frame)?;
    let (w, h) = (d.doc.width() as i32, d.doc.height() as i32);
    if a.x < 0 || a.y < 0 || a.x >= w || a.y >= h {
        return Err(format!("({}, {}) is outside the {w}x{h} canvas", a.x, a.y));
    }
    let px = super::view::pixels(d, Some(layer), frame)?;
    let at = |x: i32, y: i32| -> [u8; 4] {
        let i = (y as usize * w as usize + x as usize) * 4;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    };
    let target = at(a.x, a.y);
    let t = tolerance as u8;
    let matches = |c: [u8; 4]| (c[3] == 0 && target[3] == 0) || c.iter().zip(&target).all(|(p, q)| p.abs_diff(*q) <= t);
    let mut mask = vec![0u8; (w * h) as usize];
    if contiguous {
        let mut stack = vec![(a.x, a.y)];
        while let Some((x, y)) = stack.pop() {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let i = (y * w + x) as usize;
            if mask[i] != 0 || !matches(at(x, y)) {
                continue;
            }
            mask[i] = 255;
            stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                if matches(at(x, y)) {
                    mask[(y * w + x) as usize] = 255;
                }
            }
        }
    }
    let mut p = Paint { layer: Some(Ref::Index(layer)), frame: Some(frame), ..Default::default() };
    p.doc = a.paint.doc.clone();
    p.ink = a.paint.ink;
    p.mirror = a.paint.mirror;
    p.mirror_axis_x = a.paint.mirror_axis_x;
    p.mirror_axis_y = a.paint.mirror_axis_y;
    p.shading_ramp = a.paint.shading_ramp.clone();
    p.shading_reverse = a.paint.shading_reverse;
    paint_op(e, &p, "Fill", |d, l, f| unsafe {
        font::pixmith_stamp_mask(
            d.doc.raw(),
            l,
            f,
            0,
            0,
            w as u32,
            h as u32,
            mask.as_ptr(),
            p.ink_color(a.color.0),
            flags,
            false,
        )
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct SprayArgs {
    /// Centers of the puffs [[x, y], ...] (1-4096).
    pub points: Vec<[i32; 2]>,
    pub color: Color,
    /// Radius in pixels (default 4).
    pub radius: Option<u32>,
    /// Pixels per puff, 1-100 (default 20).
    pub density: Option<u32>,
    /// Random seed (default 1). The same seed gives the same pattern.
    pub seed: Option<u32>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn spray(e: &mut Engine, a: SprayArgs) -> R<Out> {
    if a.points.is_empty() || a.points.len() > 4096 {
        return Err("spray needs 1-4096 points".into());
    }
    let (radius, density, seed) =
        (a.radius.unwrap_or(4).clamp(1, 256), a.density.unwrap_or(20).clamp(1, 100), a.seed.unwrap_or(1));
    let flags = a.paint.flags();
    paint_op(e, &a.paint, "Spray", |d, l, f| {
        for (i, p) in a.points.iter().enumerate() {
            unsafe {
                shapes::pixmith_spray(
                    d.doc.raw(),
                    l,
                    f,
                    p[0],
                    p[1],
                    a.paint.ink_color(a.color.0),
                    radius,
                    density,
                    seed.wrapping_add(i as u32),
                    flags,
                )
            };
        }
    })
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum GradientKind {
    Linear,
    Radial,
}

#[derive(Deserialize, JsonSchema)]
pub struct GradientArgs {
    /// Where color_from is [x, y] (the center for radial).
    pub from: [i32; 2],
    /// Where color_to is reached [x, y].
    pub to: [i32; 2],
    pub color_from: Color,
    pub color_to: Color,
    /// linear (default) or radial.
    pub kind: Option<GradientKind>,
    /// 0 smooth (default), or Bayer matrix size 2, 4 or 8 for an ordered dither between the two colors.
    pub dither: Option<u32>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn gradient(e: &mut Engine, a: GradientArgs) -> R<Out> {
    let dither = a.dither.unwrap_or(0);
    if ![0, 2, 4, 8].contains(&dither) {
        return Err("dither must be 0, 2, 4 or 8".into());
    }
    let kind = a.kind.unwrap_or(GradientKind::Linear) as u32;
    paint_op(e, &a.paint, "Gradient", |d, l, f| unsafe {
        ink::pixmith_gradient(
            d.doc.raw(),
            l,
            f,
            a.from[0],
            a.from[1],
            a.to[0],
            a.to[1],
            a.color_from.0,
            a.color_to.0,
            kind,
            dither,
            false,
        )
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct TextArgs {
    pub text: String,
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    pub color: Option<Color>,
    /// Pixel scale of the 5x8 font, 1-16 (default 1).
    pub scale: Option<u32>,
    /// Only return the size, draw nothing.
    pub measure_only: Option<bool>,
    #[serde(flatten)]
    pub paint: Paint,
}

fn draw_text(e: &mut Engine, a: TextArgs) -> R<Out> {
    let scale = a.scale.unwrap_or(1).clamp(1, 16);
    let c = cstr(&a.text);
    let mut size = [0i32; 2];
    unsafe { font::pixmith_text_size(c.as_ptr(), scale, size.as_mut_ptr()) };
    if a.measure_only.unwrap_or(false) {
        return Out::json(json!({ "width": size[0], "height": size[1] }));
    }
    let color = a.color.ok_or("draw_text needs `color` (or measure_only=true)")?;
    let flags = a.paint.flags();
    let mut out = paint_op(e, &a.paint, "Text", |d, l, f| unsafe {
        font::pixmith_text(d.doc.raw(), l, f, a.x, a.y, c.as_ptr(), scale, a.paint.ink_color(color.0), flags, false)
    })?;
    if let Some(o) = out.value.as_object_mut() {
        o.insert("width".into(), json!(size[0]));
        o.insert("height".into(), json!(size[1]));
    }
    Ok(out)
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum BrushAction {
    Capture,
    Reset,
    Info,
}

#[derive(Deserialize, JsonSchema)]
pub struct BrushArgs {
    pub action: BrushAction,
    /// capture: document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// capture: layer to take the selected pixels from (default: active).
    pub layer: Option<Ref>,
    /// capture: frame (default: active).
    pub frame: Option<u32>,
}

pub fn brush_info() -> serde_json::Value {
    let mut b = [0i32; 3];
    unsafe { ink::pixmith_brush_info(b.as_mut_ptr()) };
    json!({ "custom_brush": b[0] != 0, "width": (b[0] != 0).then_some(b[1]), "height": (b[0] != 0).then_some(b[2]) })
}

fn brush(e: &mut Engine, a: BrushArgs) -> R<Out> {
    match a.action {
        BrushAction::Capture => {
            let d = e.doc(&a.doc)?;
            let l = d.image_layer(&a.layer)?;
            let f = d.frame(a.frame)?;
            crate::doc::clear_error();
            if !unsafe { ink::pixmith_brush_from_selection(d.doc.raw(), l, f) } {
                return Err(crate::doc::fail("nothing visible is selected on that layer; select pixels first"));
            }
        }
        BrushAction::Reset => ink::pixmith_brush_reset(),
        BrushAction::Info => {}
    }
    Out::json(brush_info())
}
