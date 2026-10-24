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
```

`print-config` only prints; it never edits a client's configuration.

## Client setup

Replace `/path/to/pixmith-mcp` with the real path (`which pixmith-mcp`, or use `print-config`). The optional
`--root <dir>` limits file access to one folder (recommended, see [Security](#security)).

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

49 tools in 15 groups (`pixmith-mcp list-tools` prints them; `help` topic `tools` gives the agent the same list).
Tools with many related operations take an `action` argument, which keeps the tool count well below the limits some
clients impose (Cursor, VS Code, Windsurf) while every schema stays flat and explicit.

| Group | Tools | Covers |
| --- | --- | --- |
| Documents | `doc_new`, `doc_open`, `doc_save`, `doc_close`, `doc_list`, `doc_info`, `doc_set_active` | New sprites (size, background, color mode, palette preset); open `.pxmt` / `.aseprite` / `.ase`; import PNG, animated GIF, grid sprite sheets and sheets with Aseprite JSON; save / save as / save a copy; full inspection (layer tree, frames and durations, cel occupancy, tags, palette, slices, tilesets, user data, selection, history) |
| Viewing | `render`, `get_pixels`, `get_pixel` | PNG image results (auto scale, frames, single layer or group, region, background, onion skin); exact colors as a compact character grid with legend or as hex |
| Drawing | `draw_pixels`, `draw_shape`, `draw_polygon`, `draw_curve`, `draw_stroke`, `fill`, `spray`, `gradient`, `draw_text`, `brush` | Batch pixels, line / rectangle / ellipse (filled or outline), polygons, Bezier curves, freehand strokes with pixel-perfect mode, flood fill (contiguous or global, tolerance, dither patterns), spray, linear / radial gradients with Bayer dither, the built-in pixel font; brush size and shape, ink modes (simple, alpha, lock alpha, behind, shading with a ramp), mirror symmetry, custom brushes |
| Selection | `select`, `clipboard` | Rectangle, polygon / lasso, magic wand, select by color, all / none / invert, expand / contract / border, replace / add / subtract; delete, move and crop; copy / cut / paste (also a PNG file) as a floating selection, move, commit or cancel |
| Transform | `transform`, `free_transform`, `sprite_size` | Flips, 90 / 180-degree turns, free rotation with RotSprite (per cel, all frames, all layers); a free-transform session (position, size, angle, skew, anchor); scale sprite; canvas size with anchor or offset |
| Layers & frames | `layer`, `frame`, `cel`, `tag` | Add / group / duplicate / remove / reorder / into and out of groups / merge down / properties (name, visibility, opacity, 19 blend modes) / PNG as layer; frames add / duplicate / remove / move / durations; cels link / unlink / opacity / copy / paste / move / clear; tags with direction, repeat and color |
| Color | `palette`, `color_mode` | Get, set, add, insert, remove, move, duplicate, replace, sort, ramps, presets (PICO-8, DB16/32, Endesga 32, Game Boy, NES, grays), palette from sprite, `.gpl` / `.hex` files; RGBA / grayscale / indexed with dithering |
| Filters | `filter` | Replace color, invert, desaturate, HSL, brightness/contrast, color balance, curves, outline, drop shadow, remove isolated pixels, blur, sharpen; per cel / layer / frame, clipped to the selection or a region; previews as images |
| Tilemaps | `tileset`, `tilemap`, `terrain`, `terrain_paint` | Tilesets (view as image, read and write tile pixels, add / duplicate / delete / rename); tilemap layers (create, convert from image, stamp with flips, read the grid); terrain auto-tiling: sides / blob / corners (dual grid) modes, several terrains with priorities, templates, masks, peers, symmetry, weights, terrain brush / rectangle / flood / overrides, rebuild, missing-tile report |
| Slices & user data | `slice`, `user_data` | Slices with 9-patch centers and pivots, keyed per frame; text and color user data of the sprite, layers, cels, tags and slices |
| Tweening | `tween` | Keys for x, y, rotation, scale, opacity; eases including Bezier; pivots; RotSprite and snapping options; copy / paste keys; bake into cels |
| Export | `export`, `export_godot`, `export_godot_tileset` | PNG (frame, scale), GIF, APNG, PNG sequences, sprite sheets (horizontal / vertical / grid, padding, border, extrude, trim, Aseprite JSON) per tag or per layer; Godot 4 SpriteFrames `.tres` + atlas (+ `.tscn`), Godot 4 TileSet `.tres` with terrains and collisions; dry runs and previews |
| History | `undo`, `redo`, `history`, `history_goto`, `transaction` | Every mutating call is one labeled undo step; jump anywhere in the history; group many calls into one step |
| Scripting | `run_lua` | The Lua 5.4 scripting API of Pixmith (sandboxed, one undo step, errors roll back) for anything else |
| Help | `help` | The usage guide, the Lua API reference, example scripts, palette presets, the tool list |

The server also provides:

- **Instructions** sent at initialization: the conventions (0-based coordinates, layer 0 at the bottom, colors,
  undo) and the main workflows.
- **Prompts**: `create_sprite_animation`, `export_for_godot`, `autotile_tileset`, `review_sprite`.
- **Resources**: `pixmith://docs/guide`, `pixmith://docs/lua-api`, `pixmith://docs/lua-examples`,
  `pixmith://docs/tools`, and for every open document `pixmith://documents/<id>/info` (JSON) and
  `pixmith://documents/<id>/render.png`.

### Conventions

- Documents are addressed by the id `doc_new` / `doc_open` return (`"d1"`); `doc` can be left out when only one
  document is open.
- Pixel coordinates are 0-based from the top-left. Layers are 0-based with layer 0 at the bottom; layers, tags,
  slices and tilesets can also be given by name. Frames are 0-based. (Lua scripts use 1-based layers and frames, as
  documented in the Lua API.)
- Tools without `layer` / `frame` use the document's active layer and frame, which follow newly added layers and
  frames, like the editor.
- Colors are `"#RRGGBB"` or `"#RRGGBBAA"` (also `#RGB`, `rgb()`, CSS basic names, `transparent`). Results always print
  `#rrggbbaa`.
- Errors come back as tool results with `isError: true` and the core's message; a failed call changes nothing.
  Unknown argument names are reported (with the valid ones), which catches typos early.
- Large results are bounded: `get_pixels` returns at most 16384 pixels per call (4096 as hex), `render` at most 16
  images of at most 4096 px, `history` is paginated, `doc_info` truncates very long frame lists.

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
  limits.
- `--http` binds where you tell it (use `127.0.0.1`), accepts only local `Host` headers and local browser origins,
  and has no authentication. Prefer stdio.
- The server never touches Pixmith's own settings, autosave or script folders.

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
