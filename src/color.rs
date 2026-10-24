//! Colors as tools accept and return them. The core uses `0xRRGGBBAA` integers; tools take strings like
//! `#ff8800`, `#ff880080`, `rgb(255,136,0)`, `transparent` or a raw integer, and always print `#rrggbbaa`.

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer};
use std::borrow::Cow;

/// A color, `0xRRGGBBAA`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

pub const COLOR_HELP: &str = "Color as \"#RRGGBB\" or \"#RRGGBBAA\" (also \"#RGB\", \"#RGBA\", \"rgb(r,g,b)\", \"rgba(r,g,b,a)\" with a in 0-1 or 0-255, a CSS basic name like \"red\", or \"transparent\"). A fully transparent color erases with the simple ink.";

const NAMES: &[(&str, u32)] = &[
    ("transparent", 0x0000_0000),
    ("none", 0x0000_0000),
    ("black", 0x0000_00FF),
    ("white", 0xFFFF_FFFF),
    ("red", 0xFF00_00FF),
    ("lime", 0x00FF_00FF),
    ("green", 0x0080_00FF),
    ("blue", 0x0000_FFFF),
    ("yellow", 0xFFFF_00FF),
    ("cyan", 0x00FF_FFFF),
    ("aqua", 0x00FF_FFFF),
    ("magenta", 0xFF00_FFFF),
    ("fuchsia", 0xFF00_FFFF),
    ("gray", 0x8080_80FF),
    ("grey", 0x8080_80FF),
    ("silver", 0xC0C0_C0FF),
    ("maroon", 0x8000_00FF),
    ("olive", 0x8080_00FF),
    ("navy", 0x0000_80FF),
    ("purple", 0x8000_80FF),
    ("teal", 0x0080_80FF),
    ("orange", 0xFFA5_00FF),
    ("brown", 0xA52A_2AFF),
    ("pink", 0xFFC0_CBFF),
];

impl Color {
    pub fn parse(s: &str) -> Result<Color, String> {
        let t = s.trim();
        let lower = t.to_ascii_lowercase();
        if let Some((_, v)) = NAMES.iter().find(|(n, _)| *n == lower) {
            return Ok(Color(*v));
        }
        if let Some(inner) = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb(")) {
            let inner = inner.strip_suffix(')').ok_or_else(|| format!("bad color {s:?}: missing ')'"))?;
            let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
            if parts.len() != 3 && parts.len() != 4 {
                return Err(format!("bad color {s:?}: expected 3 or 4 components"));
            }
            let mut c = [0u8, 0, 0, 255];
            for (i, p) in parts.iter().enumerate() {
                let v: f64 = p.parse().map_err(|_| format!("bad color {s:?}: {p:?} is not a number"))?;
                // CSS alpha is 0-1; larger values are read as 0-255.
                c[i] = if i == 3 && v <= 1.0 {
                    (v.clamp(0.0, 1.0) * 255.0).round() as u8
                } else {
                    v.clamp(0.0, 255.0).round() as u8
                };
            }
            return Ok(Color(u32::from_be_bytes(c)));
        }
        let hex = lower.strip_prefix('#').or_else(|| lower.strip_prefix("0x")).unwrap_or(&lower);
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(format!("bad color {s:?}: use \"#RRGGBB\" or \"#RRGGBBAA\""));
        }
        let expand = |h: &str| h.chars().flat_map(|c| [c, c]).collect::<String>();
        let full = match hex.len() {
            3 => format!("{}ff", expand(hex)),
            4 => expand(hex),
            6 => format!("{hex}ff"),
            8 => hex.to_string(),
            _ => return Err(format!("bad color {s:?}: use \"#RRGGBB\" or \"#RRGGBBAA\"")),
        };
        u32::from_str_radix(&full, 16).map(Color).map_err(|e| format!("bad color {s:?}: {e}"))
    }
}

/// `#rrggbbaa`.
pub fn hex(rgba: u32) -> String {
    format!("#{rgba:08x}")
}

/// `#rrggbb` for a tag color (`0xRRGGBB`).
pub fn hex_rgb(rgb: u32) -> String {
    format!("#{:06x}", rgb & 0xFF_FFFF)
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            S(String),
            N(u64),
        }
        match Raw::deserialize(d)? {
            Raw::S(s) => Color::parse(&s).map_err(serde::de::Error::custom),
            Raw::N(n) => {
                u32::try_from(n).map(Color).map_err(|_| serde::de::Error::custom("color integer out of range"))
            }
        }
    }
}

impl JsonSchema for Color {
    fn schema_name() -> Cow<'static, str> {
        "Color".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "description": COLOR_HELP })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_form() {
        assert_eq!(Color::parse("#f80").unwrap(), Color(0xFF8800FF));
        assert_eq!(Color::parse("#F808").unwrap(), Color(0xFF880088));
        assert_eq!(Color::parse("ff8800").unwrap(), Color(0xFF8800FF));
        assert_eq!(Color::parse("#ff880080").unwrap(), Color(0xFF880080));
        assert_eq!(Color::parse("0xff880080").unwrap(), Color(0xFF880080));
        assert_eq!(Color::parse("rgb(255, 136, 0)").unwrap(), Color(0xFF8800FF));
        assert_eq!(Color::parse("rgba(255,136,0,0.5)").unwrap(), Color(0xFF880080));
        assert_eq!(Color::parse("rgba(255,136,0,128)").unwrap(), Color(0xFF880080));
        assert_eq!(Color::parse("rgba(255,136,0,1)").unwrap(), Color(0xFF8800FF));
        assert_eq!(Color::parse("Transparent").unwrap(), Color(0));
        assert_eq!(Color::parse("red").unwrap(), Color(0xFF0000FF));
        assert!(Color::parse("#12345").is_err());
        assert!(Color::parse("blurple").is_err());
        assert_eq!(hex(0xFF8800FF), "#ff8800ff");
    }

    #[test]
    fn deserializes_strings_and_integers() {
        let c: Color = serde_json::from_str("\"#00ff00\"").unwrap();
        assert_eq!(c, Color(0x00FF00FF));
        let c: Color = serde_json::from_str("4278190335").unwrap();
        assert_eq!(c, Color(0xFF0000FF));
    }
}
