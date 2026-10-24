//! Small RGBA image helpers for the `render` family: crop, upscale, backgrounds, onion skin, PNG encoding.

/// Straight-alpha RGBA, row-major.
#[derive(Clone)]
pub struct Img {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u8>,
}

impl Img {
    pub fn new(w: usize, h: usize) -> Img {
        Img { w, h, px: vec![0; w * h * 4] }
    }

    pub fn upscale(&self, s: usize) -> Img {
        if s <= 1 {
            return self.clone();
        }
        let mut out = Img::new(self.w * s, self.h * s);
        for y in 0..out.h {
            let row = &self.px[(y / s) * self.w * 4..(y / s + 1) * self.w * 4];
            let dst = &mut out.px[y * out.w * 4..(y + 1) * out.w * 4];
            for (x, px) in row.chunks_exact(4).enumerate() {
                for k in 0..s {
                    dst[(x * s + k) * 4..(x * s + k) * 4 + 4].copy_from_slice(px);
                }
            }
        }
        out
    }

    /// Source-over: `top` drawn over `self` at `opacity` (0-1).
    pub fn draw_over(&mut self, top: &Img, opacity: f32) {
        for (d, s) in self.px.chunks_exact_mut(4).zip(top.px.chunks_exact(4)) {
            let sa = s[3] as f32 / 255.0 * opacity;
            if sa <= 0.0 {
                continue;
            }
            let da = d[3] as f32 / 255.0;
            let oa = sa + da * (1.0 - sa);
            for c in 0..3 {
                let v = (s[c] as f32 * sa + d[c] as f32 * da * (1.0 - sa)) / oa;
                d[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            d[3] = (oa * 255.0).round() as u8;
        }
    }

    /// Mixes every pixel's color halfway toward `tint` (alpha kept): onion skin coloring.
    pub fn tinted(&self, tint: [u8; 3]) -> Img {
        let mut out = self.clone();
        for p in out.px.chunks_exact_mut(4) {
            for c in 0..3 {
                p[c] = ((p[c] as u16 + tint[c] as u16) / 2) as u8;
            }
        }
        out
    }

    /// The image over an opaque background: a checkerboard of `cell`-pixel squares, or a color.
    pub fn on_background(&self, bg: Background, cell: usize) -> Img {
        let mut base = Img::new(self.w, self.h);
        match bg {
            Background::Transparent => return self.clone(),
            Background::Color(c) => {
                let b = c.to_be_bytes();
                for p in base.px.chunks_exact_mut(4) {
                    p.copy_from_slice(&b);
                }
            }
            Background::Checker => {
                let cell = cell.max(1);
                for y in 0..self.h {
                    for x in 0..self.w {
                        let v = if ((x / cell) + (y / cell)).is_multiple_of(2) { 0xEE } else { 0xC8 };
                        let i = (y * self.w + x) * 4;
                        base.px[i..i + 4].copy_from_slice(&[v, v, v, 255]);
                    }
                }
            }
        }
        base.draw_over(self, 1.0);
        base
    }

    pub fn png(&self) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, self.w as u32, self.h as u32);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(png::Compression::Fast);
            let mut w = enc.write_header().map_err(|e| e.to_string())?;
            w.write_image_data(&self.px).map_err(|e| e.to_string())?;
        }
        Ok(out)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Background {
    Transparent,
    Checker,
    Color(u32),
}

/// Decodes a PNG file into straight-alpha RGBA.
pub fn decode_png(bytes: &[u8]) -> Result<Img, String> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let src = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => src.to_vec(),
        png::ColorType::Rgb => src.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => src.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => src.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpected indexed PNG output".into()),
    };
    Ok(Img { w, h, px })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip() {
        let mut img = Img::new(2, 1);
        img.px.copy_from_slice(&[255, 0, 0, 255, 0, 0, 0, 0]);
        let big = img.upscale(3);
        assert_eq!((big.w, big.h), (6, 3));
        let back = decode_png(&big.png().unwrap()).unwrap();
        assert_eq!(back.px, big.px);
        let on = img.on_background(Background::Color(0xFFFFFFFF), 1);
        assert_eq!(&on.px[4..8], &[255, 255, 255, 255]);
    }
}
