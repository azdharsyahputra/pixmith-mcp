//! The tool catalogue. Each module defines its argument structs (which double as JSON Schemas) and handlers
//! that run on the engine thread. Adding a group means adding a module and listing its `tools()` in [`all`].

use crate::engine::{Engine, R};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

pub mod color;
pub mod docs;
pub mod draw;
pub mod export;
pub mod filters;
pub mod help;
pub mod history;
pub mod layers;
pub mod script;
pub mod select;
pub mod slices;
pub mod tilemap;
pub mod transform;
pub mod tween;
pub mod view;

/// What a tool returns: a JSON value (sent as text) and any PNG images.
#[derive(Default)]
pub struct Out {
    pub value: Value,
    pub images: Vec<Vec<u8>>,
    /// Plain text sent instead of JSON (help pages).
    pub text: Option<String>,
}

impl Out {
    pub fn json(value: Value) -> R<Out> {
        Ok(Out { value, ..Default::default() })
    }

    pub fn text(text: impl Into<String>) -> R<Out> {
        Ok(Out { text: Some(text.into()), ..Default::default() })
    }
}

#[derive(Clone, Copy)]
pub struct Hints {
    pub read_only: bool,
    pub destructive: bool,
    pub idempotent: bool,
    /// Touches files outside the server's memory.
    pub files: bool,
}

/// Reads documents only.
pub const READ: Hints = Hints { read_only: true, destructive: false, idempotent: true, files: false };
/// Adds to a document (undoable).
pub const ADD: Hints = Hints { read_only: false, destructive: false, idempotent: false, files: false };
/// Changes or removes document content (undoable).
pub const CHANGE: Hints = Hints { read_only: false, destructive: true, idempotent: false, files: false };
/// Writes or overwrites files.
pub const WRITE_FILES: Hints = Hints { read_only: false, destructive: true, idempotent: true, files: true };
/// Creates documents from files.
pub const OPEN_FILES: Hints = Hints { read_only: false, destructive: false, idempotent: false, files: true };

pub type Handler = fn(&mut Engine, Value) -> R<Out>;

pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub description: &'static str,
    pub hints: Hints,
    pub schema: fn() -> Map<String, Value>,
    pub run: Handler,
}

/// Declares a tool: `tool!(group, name, title, hints, Args, handler, description)`.
macro_rules! tool {
    ($group:literal, $name:literal, $title:literal, $hints:expr, $args:ty, $f:path, $desc:expr $(,)?) => {
        $crate::tools::ToolDef {
            name: $name,
            title: $title,
            group: $group,
            description: $desc,
            hints: $hints,
            schema: $crate::schema::input_schema::<$args>,
            run: |e, v| {
                let a: $args = $crate::tools::parse(v)?;
                $f(e, a)
            },
        }
    };
}
pub(crate) use tool;

/// Every tool, in catalogue order.
pub fn all() -> Vec<ToolDef> {
    [
        docs::tools(),
        view::tools(),
        draw::tools(),
        select::tools(),
        transform::tools(),
        layers::tools(),
        color::tools(),
        filters::tools(),
        tilemap::tools(),
        slices::tools(),
        tween::tools(),
        export::tools(),
        history::tools(),
        script::tools(),
        help::tools(),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub fn parse<T: DeserializeOwned>(v: Value) -> R<T> {
    let v = if v.is_null() { json!({}) } else { v };
    serde_json::from_value(v).map_err(|e| format!("invalid arguments: {e}"))
}

/// Rejects argument names the schema does not list (catches typos like `size` for `brush_size`).
pub fn check_unknown(schema: &Map<String, Value>, args: &Value) -> R<()> {
    let (Some(props), Some(obj)) = (schema.get("properties").and_then(Value::as_object), args.as_object()) else {
        return Ok(());
    };
    let unknown: Vec<&String> = obj.keys().filter(|k| !props.contains_key(*k)).collect();
    if unknown.is_empty() {
        return Ok(());
    }
    let mut valid: Vec<&String> = props.keys().collect();
    valid.sort();
    Err(format!(
        "unknown argument{} {}; this tool takes: {}",
        if unknown.len() > 1 { "s" } else { "" },
        unknown.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "),
        valid.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(", ")
    ))
}

/// A required option of an action-style tool.
pub fn need<T>(v: Option<T>, field: &str, action: &str) -> R<T> {
    v.ok_or_else(|| format!("action `{action}` needs `{field}`"))
}

/// An enum value as the snake_case name tools use (`MovePixels` -> `move_pixels`).
pub fn act(v: &impl std::fmt::Debug) -> String {
    let mut out = String::new();
    for (i, c) in format!("{v:?}").chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}
