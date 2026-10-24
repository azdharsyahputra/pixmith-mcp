//! The MCP server: tool listing and dispatch, prompts and resources. Tool calls run on the engine thread.

use crate::engine::EngineHandle;
use crate::tools::{self, ToolDef, check_unknown, help};
use base64::Engine as _;
use rmcp::model::*;
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler};
use serde_json::{Map, Value, json};
use std::sync::Arc;

struct Entry {
    def: ToolDef,
    schema: Map<String, Value>,
}

#[derive(Clone)]
pub struct PixmithServer {
    engine: Arc<EngineHandle>,
    tools: Arc<Vec<Entry>>,
}

impl PixmithServer {
    pub fn new(engine: Arc<EngineHandle>) -> PixmithServer {
        let tools = tools::all().into_iter().map(|def| Entry { schema: (def.schema)(), def }).collect();
        PixmithServer { engine, tools: Arc::new(tools) }
    }

    pub fn tool_list(&self) -> Vec<Tool> {
        self.tools
            .iter()
            .map(|t| {
                let h = t.def.hints;
                let ann = ToolAnnotations::with_title(t.def.title)
                    .read_only(h.read_only)
                    .destructive(h.destructive)
                    .idempotent(h.idempotent)
                    .open_world(false);
                Tool::new(t.def.name, t.def.description, Arc::new(t.schema.clone()))
                    .with_title(t.def.title)
                    .annotate(ann)
            })
            .collect()
    }

    /// Runs a tool and converts its outcome into an MCP result (errors become `isError: true` results).
    pub async fn call(&self, name: &str, args: Value) -> CallToolResult {
        let Some(i) = self.tools.iter().position(|t| t.def.name == name) else {
            let names: Vec<&str> = self.tools.iter().map(|t| t.def.name).collect();
            return CallToolResult::error(vec![ContentBlock::text(format!(
                "unknown tool {name:?}; tools: {}",
                names.join(", ")
            ))]);
        };
        if let Err(e) = check_unknown(&self.tools[i].schema, &args) {
            return CallToolResult::error(vec![ContentBlock::text(e)]);
        }
        let run = self.tools[i].def.run;
        let result = self
            .engine
            .run(move |e| {
                let before = pixmith_core::ffi::pixmith_panic_count();
                let out = run(e, args);
                let panicked = pixmith_core::ffi::pixmith_panic_count() != before;
                (out, panicked)
            })
            .await;
        match result {
            Ok((Ok(out), panicked)) => {
                let mut content = Vec::new();
                let text = match out.text {
                    Some(t) => t,
                    None => serde_json::to_string(&out.value).unwrap_or_default(),
                };
                if !text.is_empty() && text != "null" {
                    content.push(ContentBlock::text(text));
                }
                for png in out.images {
                    content
                        .push(ContentBlock::image(base64::engine::general_purpose::STANDARD.encode(png), "image/png"));
                }
                if panicked {
                    content.push(ContentBlock::text(panic_note()));
                }
                CallToolResult::success(content)
            }
            Ok((Err(msg), panicked)) => {
                let mut content = vec![ContentBlock::text(msg)];
                if panicked {
                    content.push(ContentBlock::text(panic_note()));
                }
                CallToolResult::error(content)
            }
            Err(msg) => CallToolResult::error(vec![ContentBlock::text(msg)]),
        }
    }
}

fn panic_note() -> String {
    let p = pixmith_core::ffi::pixmith_last_panic();
    let what =
        if p.is_null() { String::new() } else { unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned() };
    format!(
        "warning: the Pixmith core hit an internal error during this call ({what}); the document was rewound to its last undo step. Consider saving a copy."
    )
}

// MARK: Prompts

struct PromptDef {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    args: &'static [(&'static str, &'static str, bool)],
    body: fn(&Map<String, Value>) -> String,
}

fn arg(a: &Map<String, Value>, k: &str, d: &str) -> String {
    a.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).unwrap_or(d).to_string()
}

const PROMPTS: &[PromptDef] = &[
    PromptDef {
        name: "create_sprite_animation",
        title: "Create a sprite animation",
        description: "Step-by-step plan to draw an animated pixel-art sprite with Pixmith and export it.",
        args: &[
            ("subject", "What to draw and animate, e.g. \"a slime bouncing\"", true),
            ("size", "Canvas size, e.g. \"32x32\" (default 32x32)", false),
            ("frames", "Number of animation frames (default 6)", false),
            ("palette", "Palette preset name (see help topic palettes), e.g. \"PICO-8\"", false),
            ("output", "Where to save/export, e.g. \"art/slime\" (default: the subject's name)", false),
        ],
        body: |a| {
            let subject = arg(a, "subject", "a character");
            let size = arg(a, "size", "32x32");
            let frames = arg(a, "frames", "6");
            let palette = arg(a, "palette", "a small palette that suits the subject");
            let output = arg(a, "output", &subject.split_whitespace().last().unwrap_or("sprite").to_lowercase());
            format!(
                "Create a {size} pixel-art animation of {subject} with {frames} frames using the Pixmith tools.\n\n\
1. Read the help tool (topic guide) if you have not yet.\n\
2. doc_new with the size; load {palette} (palette load_preset, or palette replace_all with your own colors).\n\
3. Plan layers (e.g. \"outline\", \"color\", \"shading\") with layer add; keep the silhouette readable.\n\
4. Draw frame 0 with draw_shape / draw_polygon / draw_pixels / fill. Use render after each step to check, and get_pixels to verify exact pixels.\n\
5. Build the motion: frame duplicate, then change the new frame (select + select move_pixels, transform, redraw parts). Consider tween set_keys + tween bake for simple moves. Compare frames with render onion_skin.\n\
6. Set timing with frame set_duration and add a tag for the loop (tag add, direction ping_pong if it suits).\n\
7. Review render frames=[0..] for all frames; fix stray pixels.\n\
8. doc_save path=\"{output}.pxmt\" (or .aseprite), then export format=gif path=\"{output}.gif\" scale=4 and export format=sheet path=\"{output}-sheet.png\" json=true.\n\
Wrap bigger multi-call edits in transaction begin/end so each is one undo step."
            )
        },
    },
    PromptDef {
        name: "export_for_godot",
        title: "Export for Godot 4",
        description: "Export the open sprite (tags as animations) and its tilesets into a Godot 4 project.",
        args: &[
            ("project_root", "Folder that contains project.godot", true),
            ("doc", "Document id (default: the only open document)", false),
            ("subfolder", "Folder inside the project for the resources (default \"art\")", false),
        ],
        body: |a| {
            let root = arg(a, "project_root", "<project>");
            let doc = arg(a, "doc", "the open document");
            let sub = arg(a, "subfolder", "art");
            format!(
                "Export {doc} for the Godot 4 project at {root}.\n\n\
1. doc_info: check the tags (each tag becomes one animation; without tags there is a single \"default\" animation), frame durations and tilesets.\n\
2. If animations are missing, add tags (tag add) and durations (frame set_duration) first.\n\
3. export_godot project_root=\"{root}\" tres_path=\"{sub}/<name>.tres\" preview=true; read the warnings and the file list.\n\
4. Run it for real (without preview); add tscn=true for a ready AnimatedSprite2D scene, trim=true / padding=1 / extrude=true for tight atlases, save_settings=true to remember the settings in the document.\n\
5. For each tileset (doc_info tilesets): export_godot_tileset project_root=\"{root}\" tres_path=\"{sub}/<tileset>.tres\"; write_scene=true also writes the painted TileMapLayer scene.\n\
6. doc_save so the export settings are kept, and report the written files."
            )
        },
    },
    PromptDef {
        name: "autotile_tileset",
        title: "Build an auto-tiling terrain",
        description: "Create a tilemap layer with a terrain tileset and paint a map with the terrain brush.",
        args: &[
            ("tile_size", "Tile size in pixels (default 16)", false),
            ("terrain", "Terrain to make, e.g. \"grass on dirt\" (default \"grass\")", false),
            ("mode", "sides (16 tiles), blob (47 tiles) or corners (dual grid, default)", false),
        ],
        body: |a| {
            let ts = arg(a, "tile_size", "16");
            let terrain = arg(a, "terrain", "grass");
            let mode = arg(a, "mode", "corners");
            format!(
                "Build an auto-tiling {terrain} terrain with {ts}x{ts} tiles in {mode} mode.\n\n\
1. doc_new big enough for a small map (e.g. 12x8 tiles), then tilemap action=add_layer tile_width={ts}.\n\
2. terrain action=create mode={mode} name=\"{terrain}\", then terrain action=generate_template (reduced=true for fewer tiles).\n\
3. tileset action=image to see the template; paint every template tile (tileset set_tile with rows+legend, or draw on the layer where the tile is stamped) keeping each tile's edge layout.\n\
4. terrain action=assign_masks so the tiles get the template's masks; terrain get_tile to spot-check.\n\
5. terrain_paint action=rect / paint / flood to lay out the map; terrain action=missing must report 0 missing cells.\n\
6. render to review; export_godot_tileset if the map goes to Godot."
            )
        },
    },
    PromptDef {
        name: "review_sprite",
        title: "Review a sprite file",
        description: "Open a sprite file, inspect it and describe or critique it.",
        args: &[("path", "File to open (.pxmt, .aseprite, .png, .gif, sheet .json)", true)],
        body: |a| {
            let path = arg(a, "path", "<file>");
            format!(
                "Open {path} with doc_open, then doc_info and render (all frames if it is animated, a layer at a time if it has many layers). \
Describe what it shows, its palette (get_pixels / palette get), animation tags and timing, and give concrete pixel-art feedback (readability, outlines, banding, jaggies, color count). Do not modify or save the file unless asked."
            )
        },
    },
];

// MARK: Resources

const DOCS: &[(&str, &str, &str)] = &[
    ("pixmith://docs/guide", "Pixmith MCP guide", "Conventions and workflows for using the Pixmith tools"),
    ("pixmith://docs/lua-api", "Pixmith Lua API", "Reference of the Lua scripting API used by run_lua"),
    ("pixmith://docs/lua-examples", "Pixmith Lua examples", "Example Lua scripts shipped with Pixmith"),
    ("pixmith://docs/tools", "Pixmith tool catalogue", "One line per tool"),
];

fn doc_text(uri: &str) -> Option<String> {
    Some(match uri {
        "pixmith://docs/guide" => help::GUIDE.to_string(),
        "pixmith://docs/lua-api" => help::LUA_API.to_string(),
        "pixmith://docs/lua-examples" => help::examples_text(),
        "pixmith://docs/tools" => help::tools_text(),
        _ => return None,
    })
}

impl ServerHandler for PixmithServer {
    fn get_info(&self) -> ServerConfig {
        let caps = ServerCapabilities::builder().enable_tools().enable_prompts().enable_resources().build();
        ServerConfig::new(caps)
            .with_server_info(Implementation::new("pixmith-mcp", env!("CARGO_PKG_VERSION")).with_title("Pixmith"))
            .with_instructions(help::GUIDE)
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(self.tool_list()))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = request.arguments.map(Value::Object).unwrap_or_else(|| json!({}));
        Ok(self.call(&request.name, args).await.into())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let list = PROMPTS
            .iter()
            .map(|p| {
                let args = p
                    .args
                    .iter()
                    .map(|(n, d, req)| PromptArgument::new(*n).with_description(*d).with_required(*req))
                    .collect();
                Prompt::new(p.name, Some(p.description), Some(args)).with_title(p.title)
            })
            .collect();
        Ok(ListPromptsResult::with_all_items(list))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let p = PROMPTS
            .iter()
            .find(|p| p.name == request.name)
            .ok_or_else(|| McpError::invalid_params(format!("unknown prompt {:?}", request.name), None))?;
        let args = request.arguments.unwrap_or_default();
        for (n, _, req) in p.args {
            if *req && args.get(*n).and_then(Value::as_str).is_none_or(|s| s.trim().is_empty()) {
                return Err(McpError::invalid_params(format!("prompt {} needs argument {n:?}", p.name), None));
            }
        }
        let text = (p.body)(&args);
        Ok(GetPromptResult::new(vec![PromptMessage::new_text(Role::User, text)]).with_description(p.description).into())
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let mut list: Vec<Resource> = DOCS
            .iter()
            .map(|(uri, name, desc)| Resource::new(*uri, *name).with_description(*desc).with_mime_type("text/markdown"))
            .collect();
        let docs = self
            .engine
            .run(|e| e.docs.iter().map(|d| (d.id.clone(), d.name.clone())).collect::<Vec<_>>())
            .await
            .unwrap_or_default();
        for (id, name) in docs {
            list.push(
                Resource::new(format!("pixmith://documents/{id}/info"), format!("{name} ({id}) info"))
                    .with_description("doc_info of an open document")
                    .with_mime_type("application/json"),
            );
            list.push(
                Resource::new(format!("pixmith://documents/{id}/render.png"), format!("{name} ({id}) image"))
                    .with_description("The active frame, rendered")
                    .with_mime_type("image/png"),
            );
        }
        Ok(ListResourcesResult::with_all_items(list))
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult::with_all_items(vec![
            ResourceTemplate::new("pixmith://documents/{doc}/info", "Document info").with_mime_type("application/json"),
            ResourceTemplate::new("pixmith://documents/{doc}/render.png", "Document image").with_mime_type("image/png"),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri.clone();
        if let Some(text) = doc_text(&uri) {
            return Ok(ReadResourceResult::new(vec![
                ResourceContents::text(text, uri).with_mime_type("text/markdown"),
            ])
            .into());
        }
        let not_found = || McpError::resource_not_found(format!("no resource {uri:?}"), None);
        let rest = uri.strip_prefix("pixmith://documents/").ok_or_else(not_found)?;
        let (id, what) = rest.split_once('/').ok_or_else(not_found)?;
        let (tool, args) = match what {
            "info" => ("doc_info", json!({ "doc": id })),
            "render.png" => ("render", json!({ "doc": id })),
            _ => return Err(not_found()),
        };
        let r = self.call(tool, args).await;
        if r.is_error == Some(true) {
            return Err(not_found());
        }
        let mut contents = Vec::new();
        for c in r.content {
            match c {
                ContentBlock::Text(t) if what == "info" => {
                    contents.push(ResourceContents::text(t.text, uri.clone()).with_mime_type("application/json"))
                }
                ContentBlock::Image(i) => {
                    contents.push(ResourceContents::blob(i.data, uri.clone()).with_mime_type("image/png"))
                }
                _ => {}
            }
        }
        Ok(ReadResourceResult::new(contents).into())
    }
}
