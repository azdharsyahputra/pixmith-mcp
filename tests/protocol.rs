//! MCP protocol surface: initialize, tools/list schemas and annotations, prompts, resources, error handling.

mod common;
use common::Client;
use serde_json::{Value, json};

#[test]
fn tools_list_has_plain_schemas_and_annotations() {
    let mut c = Client::start(&[]);
    let r = c.request("tools/list", json!({}));
    let tools = r["result"]["tools"].as_array().expect("tools");
    assert!(tools.len() >= 45, "only {} tools", tools.len());
    let mut names = std::collections::HashSet::new();
    for t in tools {
        let name = t["name"].as_str().unwrap();
        assert!(names.insert(name.to_string()), "duplicate tool {name}");
        assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{name}");
        assert!(t["description"].as_str().unwrap().len() > 40, "{name} needs a real description");
        let s = &t["inputSchema"];
        assert_eq!(s["type"], "object", "{name}");
        assert!(s["properties"].is_object(), "{name}");
        let text = s.to_string();
        for bad in ["$ref", "null", "$defs", "definitions", "\"format\":\"", "\"type\":["] {
            assert!(!text.contains(bad), "{name} schema contains {bad}: {text}");
        }
        let a = &t["annotations"];
        assert!(
            a["readOnlyHint"].is_boolean() && a["destructiveHint"].is_boolean() && a["idempotentHint"].is_boolean(),
            "{name}: {a}"
        );
        assert_eq!(a["openWorldHint"], false);
        assert!(a["title"].is_string());
    }
    for must in [
        "doc_new",
        "doc_open",
        "doc_save",
        "doc_close",
        "doc_list",
        "doc_info",
        "render",
        "get_pixels",
        "get_pixel",
        "draw_pixels",
        "draw_shape",
        "draw_polygon",
        "draw_curve",
        "draw_stroke",
        "fill",
        "spray",
        "gradient",
        "draw_text",
        "select",
        "clipboard",
        "transform",
        "free_transform",
        "sprite_size",
        "layer",
        "frame",
        "cel",
        "tag",
        "palette",
        "color_mode",
        "filter",
        "tileset",
        "tilemap",
        "terrain",
        "terrain_paint",
        "slice",
        "user_data",
        "tween",
        "export",
        "export_godot",
        "export_godot_tileset",
        "undo",
        "redo",
        "history",
        "history_goto",
        "transaction",
        "run_lua",
        "help",
    ] {
        assert!(names.contains(must), "missing tool {must}");
    }
    let read_only: Vec<&str> = tools
        .iter()
        .filter(|t| t["annotations"]["readOnlyHint"] == true)
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for r in ["render", "get_pixels", "doc_info", "doc_list", "help", "history"] {
        assert!(read_only.contains(&r), "{r} should be read-only");
    }
    assert!(!read_only.contains(&"draw_pixels"));
}

#[test]
fn errors_are_tool_results_not_crashes() {
    let mut c = Client::start(&[]);
    let e = c.err("render", json!({}));
    assert!(e.contains("no document is open"), "{e}");
    let e = c.err("nope", json!({}));
    assert!(e.contains("unknown tool"), "{e}");
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let e = c.err("draw_shape", json!({ "shape": "line", "from": [0, 0], "to": [3, 3], "color": "#f00", "size": 3 }));
    assert!(e.contains("unknown argument `size`") && e.contains("brush_size"), "{e}");
    let e = c.err("draw_shape", json!({ "shape": "triangle", "from": [0, 0], "to": [3, 3], "color": "#f00" }));
    assert!(e.contains("invalid arguments"), "{e}");
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "#12345" }));
    assert!(e.contains("bad color"), "{e}");
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "red", "layer": 7 }));
    assert!(e.contains("layer 7 does not exist"), "{e}");
    let e = c.err("layer", json!({ "action": "remove", "layer": 0 }));
    assert!(e.contains("cannot remove"), "{e}");
    let e = c.err("doc_open", json!({ "path": "missing.aseprite" }));
    assert!(e.contains("missing.aseprite"), "{e}");
    // Two documents: `doc` becomes required.
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let e = c.err("doc_info", json!({}));
    assert!(e.contains("pass `doc`"), "{e}");
    assert_eq!(c.ok("doc_info", json!({ "doc": "d2", "sections": [] }))["width"], 4);
    // The server is still alive and well.
    assert_eq!(c.ok("doc_list", json!({}))["documents"].as_array().unwrap().len(), 2);
}

#[test]
fn prompts_and_resources() {
    let mut c = Client::start(&[]);
    let p = c.request("prompts/list", json!({}));
    let prompts = p["result"]["prompts"].as_array().unwrap();
    assert!(prompts.iter().any(|p| p["name"] == "create_sprite_animation"));
    assert!(prompts.iter().any(|p| p["name"] == "export_for_godot"));
    let g = c.request(
        "prompts/get",
        json!({ "name": "create_sprite_animation", "arguments": { "subject": "a bouncing slime" } }),
    );
    let text = g["result"]["messages"][0]["content"]["text"].as_str().unwrap();
    assert!(text.contains("bouncing slime") && text.contains("doc_new"), "{text}");
    let g = c.request("prompts/get", json!({ "name": "export_for_godot", "arguments": {} }));
    assert!(g.get("error").is_some(), "a missing required argument is an error: {g}");

    let r = c.request("resources/list", json!({}));
    let uris: Vec<&str> =
        r["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"pixmith://docs/lua-api"));
    let lua = c.request("resources/read", json!({ "uri": "pixmith://docs/lua-api" }));
    assert!(lua["result"]["contents"][0]["text"].as_str().unwrap().contains("sprite:getPixel"));
    let ex = c.request("resources/read", json!({ "uri": "pixmith://docs/lua-examples" }));
    assert!(ex["result"]["contents"][0]["text"].as_str().unwrap().contains("```lua"));

    c.ok("doc_new", json!({ "width": 8, "height": 8, "background": "#336699" }));
    let r = c.request("resources/list", json!({}));
    let uris: Vec<String> =
        r["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap().to_string()).collect();
    assert!(uris.contains(&"pixmith://documents/d1/render.png".to_string()), "{uris:?}");
    let img = c.request("resources/read", json!({ "uri": "pixmith://documents/d1/render.png" }));
    assert_eq!(img["result"]["contents"][0]["mimeType"], "image/png");
    let info = c.request("resources/read", json!({ "uri": "pixmith://documents/d1/info" }));
    let v: Value = serde_json::from_str(info["result"]["contents"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(v["width"], 8);
    let bad = c.request("resources/read", json!({ "uri": "pixmith://documents/d9/info" }));
    assert!(bad.get("error").is_some());

    let help = c.ok("help", json!({ "topic": "lua_api" }));
    assert!(help.as_str().unwrap().contains("Lua scripting reference"));
}

#[test]
fn root_confines_file_access() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("art");
    std::fs::create_dir(&root).unwrap();
    let outside = dir.path().join("outside.aseprite");
    let root_s = root.to_string_lossy().into_owned();
    let mut c = Client::start_in(dir, &["--root", &root_s]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let e = c.err("doc_save", json!({ "path": outside.to_string_lossy() }));
    assert!(e.contains("outside the allowed root"), "{e}");
    let e = c.err("export", json!({ "format": "png", "path": "../escape.png" }));
    assert!(e.contains("outside the allowed root"), "{e}");
    // Relative paths land inside the root, and missing folders are created there.
    let saved = c.ok("doc_save", json!({ "path": "sprites/hero.aseprite" }));
    assert!(saved["saved"].as_str().unwrap().ends_with("hero.aseprite"));
    assert!(root.join("sprites/hero.aseprite").is_file());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("link.aseprite")).unwrap();
        std::fs::write(&outside, b"x").unwrap();
        let e = c.err("doc_open", json!({ "path": "link.aseprite" }));
        assert!(e.contains("outside the allowed root"), "{e}");
    }
}

#[test]
fn print_config_and_list_tools() {
    let exe = env!("CARGO_BIN_EXE_pixmith-mcp");
    for client in
        ["claude-code", "claude-desktop", "codex", "antigravity", "cursor", "gemini", "vscode", "windsurf", "json"]
    {
        let out = std::process::Command::new(exe).args(["print-config", client]).output().unwrap();
        assert!(out.status.success(), "{client}");
        let s = String::from_utf8(out.stdout).unwrap();
        assert!(s.contains("pixmith-mcp"), "{client}: {s}");
    }
    let out = std::process::Command::new(exe).args(["list-tools", "--markdown"]).output().unwrap();
    let s = String::from_utf8(out.stdout).unwrap();
    assert!(s.starts_with("| Group | Tool |") && s.contains("`render`"));
    let bad = std::process::Command::new(exe).args(["print-config", "nope"]).output().unwrap();
    assert!(!bad.status.success());
}
