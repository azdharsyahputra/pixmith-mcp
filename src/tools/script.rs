//! Lua scripting: the escape hatch for anything the other tools do not cover.

use super::{CHANGE, Out, ToolDef, tool};
use crate::doc::{clear_error, take_string};
use crate::engine::{DocEntry, Engine, R, Undo};
use pixmith_core::scripting::{self, Limits};
use pixmith_core::{Editor, Sprite};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Largest script source accepted, in bytes.
const MAX_SOURCE: usize = 16 << 20;
/// `timeout_seconds` range; the default is the core's own (10 s).
const MAX_TIMEOUT_SECONDS: u64 = 120;
/// Nobody cancels a script from this server (a call blocks the engine thread until it ends).
static NO_CANCEL: AtomicBool = AtomicBool::new(false);

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Scripting",
        "run_lua",
        "Run a Lua script",
        CHANGE,
        LuaArgs,
        run_lua,
        "Run a Lua 5.4 script against a document (sandboxed: no io/os; 128 MB memory, 10 s by default, `timeout_seconds` 1-120 raises the time and the instruction budget with it; source up to 16 MiB) as ONE undo step; if it raises an error the document is left unchanged. The script sees `sprite` (pixels, layers, frames, cels, tags, slices, tilesets, selection, palette, user data), `app` and `color`; print() and app.alert() output is returned. NOTE: in Lua, layers/frames/tags are 1-based (layer 1 = bottom) while pixel coordinates and palette indices are 0-based; `sprite.activeLayer`/`activeFrame` are the document's active layer/frame. Read the full API with the help tool (topic lua_api) or the pixmith://docs/lua-api resource. Give `source` (aliases: `script`, `code`) or a .lua `path`."
    )]
}

#[derive(Deserialize, JsonSchema)]
pub struct LuaArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Lua source code (up to 16 MiB).
    pub source: Option<String>,
    /// Alias of `source`.
    pub script: Option<String>,
    /// Alias of `source`.
    pub code: Option<String>,
    /// A .lua file to run instead of `source`.
    pub path: Option<String>,
    /// Time limit in seconds, 1-120 (default 10). The instruction budget grows in proportion.
    pub timeout_seconds: Option<u32>,
}

/// The core's default limits, with the time (and the instruction budget in proportion) set to `seconds`.
fn limits(seconds: Option<u64>) -> Limits {
    let base = Limits::default();
    let Some(secs) = seconds else { return base };
    let per_second = base.instructions / base.time.as_secs().max(1);
    Limits { instructions: per_second.saturating_mul(secs), time: Duration::from_secs(secs), memory: base.memory }
}

/// Runs `src` on the document (one undo step) with the core's default limits, returning its printed output.
/// Updates the active layer/frame when the script assigned app.activeLayer / app.activeFrame.
pub fn run(d: &mut DocEntry, src: &str) -> R<String> {
    run_with_limits(d, src, Limits::default())
}

/// Like [`run`] with explicit limits (`pixmith_run_script_at` always uses the default ones, so this calls the
/// core's `scripting::run_with` the same way that entry point does).
pub fn run_with_limits(d: &mut DocEntry, src: &str, limits: Limits) -> R<String> {
    let raw = d.doc.raw();
    let (layer, frame) = (d.layer as usize, d.frame as usize);
    clear_error();
    let out = d.edit("Script", Undo::Live, |_| {
        // SAFETY: `raw` is the live editor of `d`, used only by this (the engine) thread.
        let ed: &mut Editor = unsafe { &mut *raw };
        ed.dirty.mark_all();
        let saved: Sprite = ed.sprite.clone();
        let outcome =
            match catch_unwind(AssertUnwindSafe(|| scripting::run_with(ed, src, (layer, frame), limits, &NO_CANCEL))) {
                Ok(o) => o,
                Err(_) => {
                    ed.sprite = saved;
                    return Err("internal error: the script crashed the scripting engine (document unchanged)".into());
                }
            };
        let text = outcome.messages.join("\n");
        match outcome.error {
            None => Ok((text, outcome.select)),
            Some(err) => Err(format!(
                "the script failed (document unchanged): {err}{}",
                if text.is_empty() { String::new() } else { format!("; output before the error: {text}") }
            )),
        }
    })?;
    let (text, select) = out;
    if let Some((l, f)) = select {
        d.layer = l as u32;
        d.frame = f as u32;
    }
    d.clamp_active();
    Ok(text)
}

fn run_lua(e: &mut Engine, a: LuaArgs) -> R<Out> {
    let mut sources = [("source", &a.source), ("script", &a.script), ("code", &a.code)]
        .into_iter()
        .filter_map(|(k, v)| v.as_ref().map(|s| (k, s)));
    let given = sources.next();
    if let Some((k, _)) = sources.next() {
        return Err(format!("give the Lua code once: `{}` and `{k}` are aliases", given.map_or("source", |g| g.0)));
    }
    let src = match (given, &a.path) {
        (Some((_, s)), None) => s.clone(),
        (None, Some(p)) => {
            let p = e.sandbox.read(p)?;
            let len = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
            if len > MAX_SOURCE as u64 {
                return Err(format!("the script is larger than {} MiB", MAX_SOURCE >> 20));
            }
            std::fs::read_to_string(&p).map_err(|err| format!("{}: {err}", p.display()))?
        }
        (Some(_), Some(_)) => return Err("give either `source` or `path`, not both".into()),
        (None, None) => return Err("give `source` (Lua code; aliases `script`, `code`) or `path` (a .lua file)".into()),
    };
    if src.len() > MAX_SOURCE {
        return Err(format!("the script is larger than {} MiB", MAX_SOURCE >> 20));
    }
    let secs = match a.timeout_seconds {
        None => None,
        Some(t) if (1..=MAX_TIMEOUT_SECONDS as u32).contains(&t) => Some(u64::from(t)),
        Some(t) => return Err(format!("timeout_seconds must be 1-{MAX_TIMEOUT_SECONDS}, got {t}")),
    };
    let d = e.doc(&a.doc)?;
    let (u0, _) = d.doc.history_counts();
    let output = run_with_limits(d, &src, limits(secs))?;
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
