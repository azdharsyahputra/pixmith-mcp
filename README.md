# pixmith-mcp

A [Model Context Protocol](https://modelcontextprotocol.io) server that gives AI agents the whole
[Pixmith](https://github.com/azdharsyahputra/pixmith) pixel-art editor: create and open sprites, draw, select and
transform, manage layers, frames, cels and tags, edit palettes and color modes, run FX filters, build tilemaps with
terrain auto-tiling, tween and bake animations, export PNG / GIF / APNG / sprite sheets / Godot 4 resources, undo and
redo, and run Lua scripts. `render` returns real images, so the agent can look at what it made.

It is one self-contained Rust binary. The Pixmith engine ([`pixmith-core`](https://github.com/azdharsyahputra/pixmith/tree/main/core))
runs in-process, through the same `pixmith_*` entry points the macOS and desktop apps use, so documents behave exactly
like in the app: every edit is an undo step, linked cels stay in sync, panics inside the core are caught and the
document is rewound instead of crashing the server.

When the Pixmith app is open on the same computer, the tools edit **its** documents instead: every edit appears in the
window as it happens and is a normal undo step there (see [Live mode](#live-mode-edit-the-open-pixmith-app)). With
no app running, everything works headless, exactly as described below.

Works with every MCP client that speaks stdio (Claude Code, Claude Desktop, Codex CLI, Google Antigravity, Cursor,
Gemini CLI, VS Code / GitHub Copilot, Windsurf, Cline, Zed, ...). Streamable HTTP is available with `--http`.

## Install

You need a Rust toolchain (1.99 or newer) and a C compiler (for the embedded Lua).

```sh
cargo install --git https://github.com/azdharsyahputra/pixmith-mcp
# or, from a checkout:
cargo build --release        # -> target/release/pixmith-mcp
```

Then print the configuration for your client, with the binary's absolute path already filled in:

```sh
pixmith-mcp print-config claude-code       # also: claude-desktop codex antigravity cursor gemini vscode windsurf json
pixmith-mcp print-config cursor --root ~/art
pixmith-mcp print-config cursor --live off      # never use the Pixmith app (default: auto)
```

`print-config` only prints; it never edits a client's configuration.

## Client setup

Replace `/path/to/pixmith-mcp` with the real path (`which pixmith-mcp`, or use `print-config`). The optional
`--root <dir>` limits file access to one folder (recommended, see [Security](#security)); `--live auto|on|off` controls
[live mode](#live-mode-edit-the-open-pixmith-app).

### Claude Code

```sh
claude mcp add pixmith -- /path/to/pixmith-mcp --root ~/art
# --scope user: every project; --scope project: shared through .mcp.json in the repository
```

### Claude Desktop

Settings > Developer > Edit Config (`~/Library/Application Support/Claude/claude_desktop_config.json` on macOS,
`%APPDATA%\Claude\claude_desktop_config.json` on Windows), then restart Claude Desktop:

```json
{
  "mcpServers": {
    "pixmith": { "command": "/path/to/pixmith-mcp", "args": ["--root", "/Users/me/art"] }
  }
}
```

### Codex CLI

`~/.codex/config.toml` (or `codex mcp add pixmith -- /path/to/pixmith-mcp --root /Users/me/art`):

```toml
[mcp_servers.pixmith]
command = "/path/to/pixmith-mcp"
args = ["--root", "/Users/me/art"]
```

### Google Antigravity

Agent panel > `...` > MCP Servers > Manage MCP Servers > View raw config. The file is
`~/.gemini/config/mcp_config.json` (all workspaces) or `<workspace>/.agents/mcp_config.json`:

```json
{
  "mcpServers": {
    "pixmith": { "command": "/path/to/pixmith-mcp", "args": ["--root", "/Users/me/art"] }
  }
}
```

### Cursor

`.cursor/mcp.json` in the project, or `~/.cursor/mcp.json` for every project:

```json
{
  "mcpServers": {
    "pixmith": { "command": "/path/to/pixmith-mcp", "args": ["--root", "/Users/me/art"] }
  }
}
```

### Gemini CLI

`~/.gemini/settings.json` (or `.gemini/settings.json` in a project), or `gemini mcp add pixmith /path/to/pixmith-mcp --root /Users/me/art`:

```json
{
  "mcpServers": {
    "pixmith": { "command": "/path/to/pixmith-mcp", "args": ["--root", "/Users/me/art"] }
  }
}
```

### VS Code (GitHub Copilot agent mode)

`.vscode/mcp.json` in the workspace, or "MCP: Open User Configuration". VS Code uses `servers`, not `mcpServers`:

```json
{
  "servers": {
    "pixmith": { "type": "stdio", "command": "/path/to/pixmith-mcp", "args": ["--root", "${workspaceFolder}"] }
  }
}
```

### Windsurf

Cascade > MCP servers > Configure (`~/.codeium/windsurf/mcp_config.json`, `%USERPROFILE%\.codeium\windsurf\mcp_config.json` on Windows):

```json
{
  "mcpServers": {
    "pixmith": { "command": "/path/to/pixmith-mcp", "args": ["--root", "/Users/me/art"] }
  }
}
```

### Anything else

Most other clients (Cline, Roo Code, Zed, LM Studio, ...) take the same `mcpServers` JSON (`print-config json`). For
HTTP-only clients run `pixmith-mcp --http 127.0.0.1:8765` and point them at `http://127.0.0.1:8765/mcp`.

## What an agent gets

53 tools in 16 groups (`pixmith-mcp list-tools` prints them; `help` topic `tools` gives the agent the same list).
Tools with many related operations take an `action` argument, which keeps the tool count well below the limits some
clients impose (Cursor, VS Code, Windsurf) while every schema stays flat and explicit.

| Group | Tools | Covers |
| --- | --- | --- |
| Documents | `doc_new`, `doc_open`, `doc_save`, `doc_close`, `doc_list`, `doc_info`, `doc_set_active` | New sprites (size, background, color mode, palette preset); open `.pxmt` / `.aseprite` / `.ase`; import PNG, animated GIF, grid sprite sheets and sheets with Aseprite JSON; save / save as / save a copy; full inspection (layer tree, frames and durations, cel occupancy, tags, palette, slices, tilesets, user data, selection, history); `doc_close` closes one document or, with `all_headless: true`, every headless one |
| App (live mode) | `app_status`, `app_connect`, `app_disconnect`, `game_screenshot` | Is the Pixmith app connected, which documents it has open; switch between the app and headless editing at runtime; screenshot of the running Godot game (Live Link game view, needs Pixmith 0.5.1+) |
| Viewing | `render`, `get_pixels`, `get_pixel` | PNG image results (auto scale, frames, single layer or group, region, background, onion skin); exact colors as a compact character grid with legend or as hex |
| Drawing | `draw_pixels`, `draw_shape`, `draw_polygon`, `draw_curve`, `draw_stroke`, `fill`, `spray`, `gradient`, `draw_text`, `brush` | Batch pixels, line / rectangle / ellipse (filled or outline), polygons, Bezier curves, freehand strokes with pixel-perfect mode, flood fill (contiguous or global, tolerance, dither patterns), spray, linear / radial gradients with Bayer dither, the built-in pixel font; brush size and shape, ink modes (simple, alpha, lock alpha, behind, shading with a ramp), mirror symmetry, custom brushes |
| Selection | `select`, `clipboard` | Rectangle, polygon / lasso, magic wand, select by color, all / none / invert, expand / contract / border, replace / add / subtract; delete, move and crop; copy / cut / paste (also a PNG file) as a floating selection, move, commit or cancel |
| Transform | `transform`, `free_transform`, `sprite_size` | Flips, 90 / 180-degree turns, free rotation with RotSprite (per cel, all frames, all layers); a free-transform session (position, size, angle, skew, anchor); scale sprite; canvas size with anchor or offset |
| Layers & frames | `layer`, `frame`, `cel`, `tag` | Add / group / duplicate / remove / reorder / into and out of groups / merge down / properties (name, visibility, opacity, 19 blend modes) / PNG as a new layer or, with `replace: true`, as the new pixels of an existing layer in one undo step; frames add / duplicate / remove / move / durations; cels link / unlink / opacity / copy / paste / move / clear; tags with direction, repeat and color |
| Color | `palette`, `color_mode` | Get, set, add, insert, remove, move, duplicate, replace, sort, ramps, presets (PICO-8, DB16/32, Endesga 32, Game Boy, NES, grays), palette from sprite, `.gpl` / `.hex` files; RGBA / grayscale / indexed with dithering |
| Filters | `filter` | Replace color, invert, desaturate, HSL, brightness/contrast, color balance, curves, outline, drop shadow, remove isolated pixels, blur, sharpen; per cel / layer / frame, clipped to the selection or a region; previews as images |
| Tilemaps | `tileset`, `tilemap`, `terrain`, `terrain_paint` | Tilesets (view as image, read and write tile pixels, add / duplicate / delete / rename); tilemap layers (create, convert from image, stamp with flips, read the grid); terrain auto-tiling: sides / blob / corners (dual grid) modes, several terrains with priorities, templates, masks, peers, symmetry, weights, terrain brush / rectangle / flood / overrides, rebuild, missing-tile report |
| Slices & user data | `slice`, `user_data` | Slices with 9-patch centers and pivots, keyed per frame; text and color user data of the sprite, layers, cels, tags and slices |
| Tweening | `tween` | Keys for x, y, rotation, scale, opacity; eases including Bezier; pivots; RotSprite and snapping options; copy / paste keys; bake into cels |
| Export | `export`, `export_godot`, `export_godot_tileset` | PNG (frame, scale), GIF, APNG, PNG sequences, sprite sheets (horizontal / vertical / grid, padding, border, extrude, trim, Aseprite JSON) per tag or per layer; Godot 4 SpriteFrames `.tres` + atlas (+ `.tscn`, and Live Link: the running game updates after every edit), Godot 4 TileSet `.tres` with terrains and collisions; dry runs and previews |
| History | `undo`, `redo`, `history`, `history_goto`, `transaction` | Every mutating call is one labeled undo step; jump anywhere in the history; group many calls into one step |
| Scripting | `run_lua` | The Lua 5.4 scripting API of Pixmith, Aseprite-compatible (`Color`, `Image`, `Sprite`, `app.command`, …; one undo step, errors roll back) for anything else; sources up to 16 MiB, `timeout_seconds` 1-120 (default 10). Files (`io`, `os`, `app.fs`, `Image{fromFile}`, `app.open`, `saveAs`) only inside `allow_paths` (folders inside `--root`); documents the script creates come back as new headless documents in `new_documents` |
| Help | `help` | The usage guide, the Lua API reference, example scripts, palette presets, the tool list, and a short how-to per tool group (`documents`, `layers`, `drawing`, `selection`, `transform`, `filters`, `tilemaps`, `slices`, `tween`, `export`, `history`, `live`) |

The server also provides:

- **Instructions** sent at initialization: the conventions (0-based coordinates, layer 0 at the bottom, colors,
  undo) and the main workflows.
- **Prompts**: `create_sprite_animation`, `export_for_godot`, `autotile_tileset`, `review_sprite`.
- **Resources**: `pixmith://docs/guide`, `pixmith://docs/lua-api`, `pixmith://docs/lua-examples`,
  `pixmith://docs/tools`, and for every open document `pixmith://documents/<id>/info` (JSON) and
  `pixmith://documents/<id>/render.png`.

### Conventions

- Documents are addressed by the id `doc_new` / `doc_open` return (`"d1"`, or `"app:doc-3"` for a document open in the
  Pixmith app); `doc` can be left out when only one document is open (with the app connected: the app's active one).
- Pixel coordinates are 0-based from the top-left. Layers are 0-based with layer 0 at the bottom; layers, tags,
  slices and tilesets can also be given by name. Frames are 0-based. (Lua scripts use 1-based layers and frames, as
  documented in the Lua API.)
- Tools without `layer` / `frame` use the document's active layer and frame, which follow newly added layers and
  frames, like the editor.
- Colors are `"#RRGGBB"` or `"#RRGGBBAA"` (also `#RGB`, `rgb()`, CSS basic names, `transparent`). Results always print
  `#rrggbbaa`.
- Errors come back as tool results with `isError: true` and the core's message; a failed call changes nothing.
  Unknown argument names and unknown actions are reported with the valid ones, which catches typos early.
- A few common synonyms are accepted: `delete` for the `remove` action of `layer`, `frame`, `tag`, `slice`,
  `palette` and `terrain` (and `clear` of `cel`: `remove` / `delete`), `delete_key` for `tween` `remove_key`,
  `remove_tile` for `tileset` `delete_tile`, and `script` / `code` for the `source` argument of `run_lua`. The tool
  schemas list the canonical names only.
- Headless documents (ids `d1`, ...) stay until closed. `doc_close {"all_headless": true}` closes them all at once
  (documents with unsaved changes are kept and listed unless `discard_changes` is true). Past 32 open headless
  documents, `doc_new` / `doc_open` close the oldest unmodified ones (saved or never edited, never the new one) and say
  so in the result's `note`.
- `layer` with `action: "import_png"` adds the PNG as a new layer; with `replace: true` it puts the PNG into the
  existing `layer` at `frame` instead (one undo step; the layer keeps name, order, opacity and blend mode; `x` / `y`
  shift it, `name` renames the layer).
- Large results are bounded: `get_pixels` returns at most 16384 pixels per call (4096 as hex), `render` at most 16
  images of at most 4096 px, `history` is paginated, `doc_info` truncates very long frame lists.

## Live mode: edit the open Pixmith app

Start Pixmith 0.5.0 or later (macOS app, or the Windows / Linux / macOS desktop app) and use the tools as usual: the agent's
edits show up in the open window while it works, each call is one undo step named **AI: \<tool\>** (for example
`AI: draw_shape rectangle`) that you can take back with Cmd/Ctrl+Z, and the document is marked as edited. Without an app
(never started, or after `app_disconnect`) the very same tools work headless on files, as before; an app that
disappears in the middle of a session is reported as an error instead of being silently replaced by headless editing
(see "No silent fallback" below). No configuration is needed.

```
agent --MCP--> pixmith-mcp --127.0.0.1, token--> Pixmith app (your open documents)
```

How it works, in short: the app writes a small discovery file (`<app data>/Live/<pid>.json`) while it runs and listens
on a loopback port. For each editing call the server fetches the document as `.pxmt` bytes, runs the unchanged tool
code on its copy with the same `pixmith-core` the app uses, and sends the result back as one undo step. The protocol is
specified in the Pixmith repository (`docs/LIVE_PROTOCOL.md`). A revision number per document stops the server from
overwriting something you changed in the meantime: it fetches again and redoes the edit on your version (up to 3
times); while a dialog is open the app asks it to wait and it retries for a few seconds.

- **Which document?** Documents open in the app have the ids `app:<id>` (`app_status` or `doc_list` shows them).
  Leave `doc` out and the tools use the app's **active** document, so you choose the target by switching tabs.
  `doc_new` and `doc_open` create the sprite in a new tab of the app; pass `"target": "headless"` to keep a document
  inside the server only (its id is `d1`, ...). `doc_save`, `doc_close` and `doc_set_active` act on the app's tab
  (`doc_save` saves with the app, so its dirty marker is cleared; `copy: true` writes a copy without touching the tab).
- **Layer and frame.** Tools without `layer` / `frame` use the app's current layer and frame. A tool aimed at another
  layer or frame, or one that adds a layer, moves the app's current layer / frame there, so you see where it works.
- **No silent fallback.** Once this server has been connected to the app in a session, a call that creates or edits a
  document never quietly continues headless when the app becomes unreachable (it quit, crashed, or dropped the
  socket): the server reconnects once, and if that fails the call returns an error ("the Pixmith app was connected but
  is not reachable now (...); start it again, or pass `target: "headless"` / call `app_disconnect` to work headless on
  purpose"). Explicit headless work keeps working: `target: "headless"` on `doc_new` / `doc_open`, a headless id (`d1`)
  as `doc`, or `app_disconnect`. Results of headless creations say `"target": "headless"` once an app was involved,
  and so do calls without `doc` that land on a headless document while the app is connected. A server that was never
  connected behaves as before (headless, no extra fields).
- **One call at a time.** Every tool call runs on the single engine thread, so parallel tool calls from a client
  (several `doc_new` at once, say) queue up and their requests to the app never interleave.
- **Modes.** `--live auto` (default) uses the app when one is running and falls back to headless otherwise (until it
  has been connected once, see above);
  `--live on` requires the app (tools fail with a clear message when none is running; `target: "headless"` still
  works); `--live off` never talks to an app. The `PIXMITH_LIVE` environment variable (`auto|on|off`) is the same
  switch; the flag wins. At runtime, `app_status` shows the state, `app_disconnect` makes the server work headless
  until `app_connect` (which also picks an instance with `pid` when several apps run, newest by default).
- **Several clients, several apps.** The newest running instance wins. Discovery files of dead processes are ignored
  (and deleted); a file whose port refuses the connection is ignored. `PIXMITH_SUPPORT_DIR` replaces the platform's
  app data folder here exactly as it does in the apps (the folder is `Pixmith` inside it, `pixmith` on Linux).
- **Limits.** `history` and `history_goto` are not available on app documents and `transaction` does nothing there: the
  app owns the undo history and every call is its own step. `undo` / `redo` are handed to the app when it supports
  `documents.undo` / `documents.redo`; otherwise the tool says to press Cmd/Ctrl+Z in the app. The app's own
  selection is not visible to the tools; a selection made with the `select` tool is kept by the server for the next
  calls and dropped as soon as the document changes in the app. Each call is a round trip and an undo step, so prefer
  batch tools (`draw_pixels`, `draw_polygon`, `run_lua`) over thousands of tiny calls.

### Godot Live Link and the game view

Needs Pixmith 0.5.1 or later. With the app connected, a Godot game can show the sprite you are drawing, live:

1. Export once with `export_godot` and `save_settings: true` (or **Export > Godot SpriteFrames** in the app). The
   document then knows its `res://` resource, so the app can tell the game which SpriteFrames it feeds.
2. Run a game that has the Live Link addon (`godot/addons/pixmith_live_link` in the Pixmith repository, copied into the
   project's `addons` folder and enabled) from Godot as a debug build. After every edit, including each tool call of
   the agent, the game swaps in the new atlas and regions in place: no re-export, no restart. Released builds never do
   this.
3. `game_screenshot` returns a screenshot of the running game as an image, so the agent can edit a sprite and then look
   at it in the game. Arguments: `timeout_ms` (default 3000, at most 10000) and `scale` (integer 1-8, nearest neighbour,
   long side capped at 2048 px) to enlarge small pixel games. It needs live mode; without the app, or without a
   running game that has the addon, it fails with a tool error that says so.

```jsonc
export_godot     {"project_root": "~/games/demo", "tres_path": "art/slime.tres", "save_settings": true}   // once
draw_pixels      {"points": [[13, 17], [18, 17]], "color": "#ff004d"}    // the running game shows it right away
game_screenshot  {"scale": 2}                                           // -> image of the game
```

### Troubleshooting live mode

- `app_status` says `connected: false` and gives the reason. Is the app running? In the app, **Preferences > Allow AI
  agents to control Pixmith (Live mode)** must be on (it is by default); turning it off closes the port and removes the
  discovery file. While an agent is connected, the app shows an "AI connected" chip in the title row; its menu lists
  the client and has **Disconnect**.
- The app was started after the agent: nothing to do in `auto` mode, the next call finds it; or call `app_connect`.
- "the Pixmith app was connected but is not reachable now": the app quit or dropped the connection mid-session. Start
  it again (the next call reconnects), or deliberately work headless with `app_disconnect` or `target: "headless"`.
  `app_status` shows the reason; before this check, such calls silently created headless documents.
- "refused the connection" / "protocol": the app and `pixmith-mcp` are from incompatible versions; update the older one.
- Running the server in a different account, container or sandbox than the app: the discovery folder and the loopback
  port must be reachable; set `PIXMITH_SUPPORT_DIR` to the app's folder if it is relocated.
- `game_screenshot` says no game answered: start the game from Godot as a debug build with the addon enabled, and make
  sure the Pixmith app is 0.5.1 or later; a game that is not running or paused in the debugger sends no frame.
- You do not want the agent to touch the app: start the server with `--live off` (`print-config <client> --live off`
  writes it into the client configuration).
- "busy" errors mean a dialog is open in the app or you are in the middle of a stroke; the server already waited a few
  seconds. Close the dialog and ask again.

## Examples

What a session looks like (tool calls as an agent makes them):

```jsonc
doc_new        {"width": 32, "height": 32, "palette_preset": "PICO-8"}          // -> {"doc": "d1", ...}
layer          {"action": "add", "name": "body"}
draw_shape     {"shape": "ellipse", "from": [8, 10], "to": [23, 27], "color": "#00e436", "filled": true}
draw_shape     {"shape": "ellipse", "from": [8, 10], "to": [23, 27], "color": "#1d2b53"}
draw_pixels    {"points": [[13, 17], [18, 17]], "color": "#1d2b53"}
render         {}                                                                // -> PNG image
frame          {"action": "add", "count": 3}
tween          {"action": "set_keys", "frame": 0, "values": {"y": 0}}
tween          {"action": "set_keys", "frame": 3, "values": {"y": -3}, "ease": "out_quad"}
tween          {"action": "bake"}                                                // optional: keys -> editable cels
tag            {"action": "add", "name": "bounce", "from": 0, "to": 3, "direction": "ping_pong"}
doc_save       {"path": "slime.pxmt"}
export         {"format": "gif", "path": "slime.gif", "scale": 4}
export_godot   {"project_root": "~/games/demo", "tres_path": "art/slime.tres", "tscn": true}
```

Or just ask: *"Make a 32x32 slime with a 4-frame bounce, export it as a GIF and into my Godot project at
~/games/demo"*, and the agent picks the tools (the `create_sprite_animation` prompt spells out a good plan).

### Pixmith documents (`.pxmt`)

`doc_open` and `doc_save` go through the core's own `pixmith_open` / `pixmith_save`: a path ending in `.pxmt` is
saved in Pixmith's native format (full sprite plus an embedded thumbnail, see the
[format spec](https://github.com/azdharsyahputra/pixmith/blob/main/docs/PXMT_FORMAT.md)); any other path is saved as
`.aseprite`. Opening detects the format from the file's content. Tools for Pixmith project folders (`project_*`) will
follow.

## Security

- The server reads and writes files only when a tool asks for it (open, save, export, palette files, PNG import,
  Lua files). Start it with `--root <dir>` to confine all of that to one folder: every path is resolved (symbolic
  links included) and rejected when it leaves the root; relative paths start at the root. Missing folders inside the
  root are created on save / export.
- Without `--root`, the agent can read and overwrite any file your user account can. Clients usually ask before a
  tool runs; the tool annotations (`readOnlyHint`, `destructiveHint`, `idempotentHint`) let them tell harmless calls
  from writes.
- Lua scripts run in Pixmith's sandbox: no `io`, `os`, `require` or bytecode, with instruction, time and memory
  limits (10 s by default, `timeout_seconds` raises it up to 120 s and the instruction budget with it; source up to
  16 MiB).
- `--http` binds where you tell it (use `127.0.0.1`), accepts only local `Host` headers and local browser origins,
  and has no authentication. Prefer stdio.
- The server never touches Pixmith's own settings, autosave or script folders. In live mode it only reads the
  discovery files in the app's `Live` folder and talks to the app on `127.0.0.1` with the per-launch token from that
  file (readable by your user only); `--live off` disables all of it. File paths given to `doc_open` / `doc_save` for
  app documents go through `--root` like every other path.

## Development

```sh
cargo test                                   # unit tests + the binary driven over stdio for every tool group
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings
npx @modelcontextprotocol/inspector --cli target/debug/pixmith-mcp --method tools/list
```

`pixmith-core` comes from the Pixmith repository, pinned to a commit in `Cargo.toml`; uncomment the `[patch]` section
there to build against a local checkout. See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT, see [LICENSE](LICENSE). `docs/LUA_API.md` is copied from the Pixmith repository (same license and author).
