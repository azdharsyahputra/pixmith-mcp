//! Lua scripting: the escape hatch for anything the other tools do not cover.

use super::{CHANGE, Out, ToolDef, tool};
use crate::doc::{clear_error, cstr, fail, take_string};
use crate::engine::{DocEntry, Engine, R, Undo};
use pixmith_core::scripting;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Scripting",
        "run_lua",
        "Run a Lua script",
        CHANGE,
        LuaArgs,
        run_lua,
        "Run a Lua 5.4 script against a document (sandboxed: no io/os; 10 s / 128 MB limits) as ONE undo step; if it raises an error the document is left unchanged. The script sees `sprite` (pixels, layers, frames, cels, tags, slices, tilesets, selection, palette, user data), `app` and `color`; print() and app.alert() output is returned. NOTE: in Lua, layers/frames/tags are 1-based (layer 1 = bottom) while pixel coordinates and palette indices are 0-based; `sprite.activeLayer`/`activeFrame` are the document's active layer/frame. Read the full API with the help tool (topic lua_api) or the pixmith://docs/lua-api resource. Give `source` or a .lua `path`."
    )]
}

#[derive(Deserialize, JsonSchema)]
pub struct LuaArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Lua source code.
    pub source: Option<String>,
    /// A .lua file to run instead of `source`.
    pub path: Option<String>,
}

/// Runs `src` on the document (one undo step), returning its printed output. Updates the active layer/frame
/// when the script assigned app.activeLayer / app.activeFrame.
pub fn run(d: &mut DocEntry, src: &str) -> R<String> {
    let c = cstr(src);
    let raw = d.doc.raw();
    let (layer, frame) = (d.layer, d.frame);
    scripting::pixmith_script_cancel(false);
    clear_error();
    let out = d.edit("Script", Undo::Live, |_| {
        let mut msg = std::ptr::null_mut();
        let ok = unsafe { scripting::pixmith_run_script_at(raw, c.as_ptr(), layer, frame, &mut msg) };
        let text = unsafe { take_string(msg) };
        if ok {
            Ok(text)
        } else {
            Err(fail(format!(
                "the script failed (document unchanged){}",
                if text.is_empty() { String::new() } else { format!("; output before the error: {text}") }
            )))
        }
    })?;
    let mut lines = Vec::new();
    for l in out.lines() {
        if let Some(rest) = l.strip_prefix("\u{1}PIXMITH-SELECT ") {
            for part in rest.split_whitespace() {
                if let Some(v) = part.strip_prefix("layer=").and_then(|v| v.parse::<u32>().ok()) {
                    d.layer = v.saturating_sub(1);
                }
                if let Some(v) = part.strip_prefix("frame=").and_then(|v| v.parse::<u32>().ok()) {
                    d.frame = v.saturating_sub(1);
                }
            }
        } else {
            lines.push(l);
        }
    }
    d.clamp_active();
    Ok(lines.join("\n"))
}

fn run_lua(e: &mut Engine, a: LuaArgs) -> R<Out> {
    let src = match (&a.source, &a.path) {
        (Some(s), None) => s.clone(),
        (None, Some(p)) => {
            let p = e.sandbox.read(p)?;
            std::fs::read_to_string(&p).map_err(|err| format!("{}: {err}", p.display()))?
        }
        (Some(_), Some(_)) => return Err("give either `source` or `path`, not both".into()),
        (None, None) => return Err("give `source` (Lua code) or `path` (a .lua file)".into()),
    };
    if src.len() > 1 << 20 {
        return Err("the script is larger than 1 MB".into());
    }
    let d = e.doc(&a.doc)?;
    let (u0, _) = d.doc.history_counts();
    let output = run(d, &src)?;
    let (u1, _) = d.doc.history_counts();
    let output: String = output.chars().take(50_000).collect();
    Out::json(json!({
        "doc": d.id, "ok": true, "changed_document": u1 != u0, "output": output,
        "active_layer": d.layer, "active_frame": d.frame,
        "layers": d.layer_count(), "frames": d.doc.frame_count(), "width": d.doc.width(), "height": d.doc.height(),
    }))
}

/// The example scripts that ship with the core: (file name, source).
pub fn examples() -> Vec<(String, String)> {
    (0..scripting::pixmith_example_script_count())
        .map(|i| unsafe {
            (
                take_string(scripting::pixmith_example_script_name(i)),
                take_string(scripting::pixmith_example_script_source(i)),
            )
        })
        .filter(|(n, _)| !n.is_empty())
        .collect()
}
