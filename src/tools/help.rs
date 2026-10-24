//! Built-in documentation, also served as MCP resources and as the server instructions.

use super::{Out, READ, ToolDef, tool};
use crate::engine::{Engine, R};
use schemars::JsonSchema;
use serde::Deserialize;

pub const LUA_API: &str = include_str!("../../docs/LUA_API.md");

/// Sent as the server's `instructions` and returned by `help` (topic guide).
pub const GUIDE: &str = r##"# Pixmith MCP: how to use it

Pixmith is a pixel-art editor (sprites, animation, tilemaps; .pxmt and .aseprite files). This server runs its
engine in-process; every tool call edits real documents with the same code the desktop apps use.

## Conventions
- Documents have ids ("d1", "d2", ...) from doc_new / doc_open (in the Pixmith app: "app:<id>", see Live mode).
  `doc` may be omitted when only one is open (with the app connected: the app's active document).
- Pixel coordinates are 0-based from the top-left. Rectangles are {x, y, w, h}; shape corners are inclusive.
- Layers are 0-based with 0 at the BOTTOM of the stack; a group appears right below its children. Layers can
  also be addressed by exact name. Frames are 0-based. (Lua scripts are 1-based for layers/frames/tags.)
- Tools without `layer`/`frame` use the document's active layer/frame; new layers/frames become active.
  doc_set_active changes them. Check doc_info when unsure.
- Colors: "#RRGGBB" or "#RRGGBBAA" (also "#RGB", "rgb(...)", CSS basic names, "transparent"). Results print
  "#rrggbbaa". The default "simple" ink replaces pixels; a transparent color erases.
- Every mutating call is exactly one undo step (undo / redo / history). Wrap a multi-call edit in
  transaction begin/end to make it one step. Failed calls change nothing.
- While a selection exists, drawing, fills, gradients and filters only touch selected pixels. select none clears it.

## Live mode (the Pixmith app)
- When the Pixmith app is open on this computer, the tools edit ITS documents: the user watches every edit appear in
  the window, and each call is one undo step named "AI: <tool>" that they can take back with Cmd/Ctrl+Z. Without
  the app everything works headless, in documents kept inside this server (ids "d1", "d2", ...).
- app_status tells which one you have: connected or not, the app's documents (ids "app:<id>", e.g. "app:doc-3") and
  the active one. app_connect / app_disconnect switch at runtime (use app_connect after the user starts the app).
- In live mode `doc` may be omitted to mean the app's ACTIVE document, so the user chooses the target by switching
  tabs. doc_new / doc_open create the sprite in the app (target "headless" keeps it inside the server instead);
  doc_save / doc_close / doc_set_active act on the app's tab. doc_list shows both kinds.
- Tools default to the app's current layer and frame; aiming a tool at another layer or frame moves the app's
  current one there, so the user sees where you are working. The user may edit at the same time: a call that lost
  that race is repeated on the new version automatically, and a busy app (open dialog) is retried for a few seconds.
- Not available on app documents: history, history_goto (the app owns the undo history; transaction does nothing,
  every call is its own step). undo / redo work when the app supports them and otherwise tell the user to press
  Cmd/Ctrl+Z. The app's own selection is not visible to tools; a selection made with the select tool lives in this
  server until someone else changes the document.
- Prefer a few meaningful calls over thousands of tiny ones: every call is a full round trip to the window and an
  undo step. draw_pixels (batches), draw_polygon and run_lua are good for bigger drawings.

## Looking at your work
- render returns a PNG you can see (auto-scaled, checkerboard = transparent). Render after each meaningful step.
- get_pixels returns exact colors as a character grid + legend (great for checking outlines and palettes).
- doc_info lists layers, frames, cels (which have pixels), tags, palette, slices, tilesets, selection, history.

## Typical workflows
- New sprite: doc_new (width, height, optionally palette_preset) -> palette (load_preset / replace_all) ->
  layer add (e.g. "outline", "color") -> draw_* / fill -> render -> doc_save path=... .pxmt or .aseprite.
- Animation: draw frame 0 -> frame duplicate -> edit the new frame -> repeat -> frame set_duration ->
  tag add (name, from, to, direction) -> render frames=[...] or onion_skin to compare -> export gif / sheet.
- Tween instead of redrawing: tween set_keys at two frames (x/y/rotation/scale/opacity, ease); render and
  exports show the motion; tween bake turns it into editable cels.
- Tilemap: tilemap add_layer (tile size) -> draw tiles via tileset set_tile or by drawing on the layer ->
  tilemap stamp. Auto-tiling: terrain create (mode) -> terrain generate_template -> paint the template
  tiles -> terrain assign_masks -> terrain_paint paint/rect/flood -> terrain missing to check coverage.
- Godot: export_godot (project_root with project.godot, tres_path inside it; one animation per tag) and
  export_godot_tileset for tilesets with terrains. preview=true shows what would be written.
- Anything else: run_lua (see help topic lua_api).
"##;

pub fn tools() -> Vec<ToolDef> {
    vec![tool!(
        "Help",
        "help",
        "Pixmith help",
        READ,
        HelpArgs,
        help,
        "Documentation for using this server: guide (conventions and workflows — read this first), lua_api (the full Lua scripting reference for run_lua), lua_examples (example scripts shipped with Pixmith), palettes (built-in palette preset names), tools (one line per tool)."
    )]
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum Topic {
    Guide,
    LuaApi,
    LuaExamples,
    Palettes,
    Tools,
}

#[derive(Deserialize, JsonSchema)]
pub struct HelpArgs {
    /// guide (default), lua_api, lua_examples, palettes or tools.
    pub topic: Option<Topic>,
}

pub fn examples_text() -> String {
    let mut out =
        String::from("# Example Lua scripts shipped with Pixmith\n\nRun one with run_lua (source = the code).\n");
    for (name, src) in super::script::examples() {
        out.push_str(&format!("\n## {name}\n\n```lua\n{}\n```\n", src.trim_end()));
    }
    out
}

pub fn tools_text() -> String {
    let mut out = String::from("# Tools\n");
    let mut group = "";
    for t in super::all() {
        if t.group != group {
            group = t.group;
            out.push_str(&format!("\n## {group}\n"));
        }
        let first = t.description.split(". ").next().unwrap_or(t.description);
        out.push_str(&format!("- {}: {}\n", t.name, first.trim_end_matches('.')));
    }
    out
}

fn help(_: &mut Engine, a: HelpArgs) -> R<Out> {
    match a.topic.unwrap_or(Topic::Guide) {
        Topic::Guide => Out::text(GUIDE),
        Topic::LuaApi => Out::text(LUA_API),
        Topic::LuaExamples => Out::text(examples_text()),
        Topic::Palettes => Out::text(format!(
            "Built-in palette presets (palette action load_preset, or doc_new palette_preset):\n- {}",
            super::docs::preset_names().join("\n- ")
        )),
        Topic::Tools => Out::text(tools_text()),
    }
}
