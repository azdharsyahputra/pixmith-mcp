//! Headless behaviour added after long agent sessions: closing and limiting headless documents, replacing a layer's
//! pixels from a PNG, action and argument aliases, help topics, and the `run_lua` limits.

mod common;
use common::{Client, write_png};
use serde_json::json;
use std::time::Instant;

fn undo_steps(c: &mut Client) -> u64 {
    c.ok("history", json!({}))["undo_steps"].as_u64().unwrap()
}

fn doc_ids(c: &mut Client) -> Vec<String> {
    c.ok("doc_list", json!({}))["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["doc"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn doc_close_all_headless_closes_every_headless_document() {
    let mut c = Client::start(&[]);
    for _ in 0..4 {
        c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    }
    c.ok("draw_pixels", json!({ "doc": "d3", "points": [[0, 0]], "color": "#f00" }));
    // The edited one stays and is reported; the others go.
    let r = c.ok("doc_close", json!({ "all_headless": true }));
    assert_eq!(r["closed"], json!(["d1", "d2", "d4"]), "{r}");
    assert_eq!(r["kept_unsaved"], json!(["d3"]), "{r}");
    assert!(r["note"].as_str().unwrap().contains("unsaved"), "{r}");
    assert_eq!(doc_ids(&mut c), ["d3"]);
    let r = c.ok("doc_close", json!({ "all_headless": true, "discard_changes": true }));
    assert_eq!(r["closed"], json!(["d3"]), "{r}");
    assert_eq!(r["open"], json!([]));
    // Nothing open is fine.
    assert_eq!(c.ok("doc_close", json!({ "all_headless": true }))["closed"], json!([]));
    // A single document and `all_headless` do not mix.
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let e = c.err("doc_close", json!({ "all_headless": true, "doc": "d5" }));
    assert!(e.contains("either `doc` or `all_headless`"), "{e}");
    assert_eq!(doc_ids(&mut c), ["d5"]);
}

#[test]
fn more_than_32_headless_documents_close_the_oldest_unmodified_ones() {
    let mut c = Client::start(&[]);
    let mut notes = Vec::new();
    for i in 1..=34 {
        let r = c.ok("doc_new", json!({ "width": 2, "height": 2 }));
        assert_eq!(r["doc"], format!("d{i}"));
        if i == 2 {
            // Edited and unsaved: must survive the cleanup.
            c.ok("draw_pixels", json!({ "doc": "d2", "points": [[0, 0]], "color": "#f00" }));
        }
        if let Some(n) = r["note"].as_str() {
            notes.push((i, n.to_string(), r["closed_headless"].clone()));
        }
    }
    let ids = doc_ids(&mut c);
    assert_eq!(ids.len(), 32, "{ids:?}");
    assert!(ids.contains(&"d2".to_string()), "the modified document is kept: {ids:?}");
    assert!(!ids.contains(&"d1".to_string()), "the oldest unmodified one is gone: {ids:?}");
    assert!(ids.contains(&"d34".to_string()));
    // The 33rd document pushed d1 out, the 34th pushed out d3 (d2 is modified); each result says so.
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0].0, 33);
    assert_eq!(notes[0].2, json!(["d1"]));
    assert!(notes[0].1.contains("d1") && notes[0].1.contains("all_headless"), "{notes:?}");
    assert_eq!(notes[1].2, json!(["d3"]));
    // A saved document counts as unmodified.
    let path = c.p("keep.pxmt");
    c.ok("doc_save", json!({ "doc": "d2", "path": path }));
    let r = c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    assert_eq!(r["closed_headless"], json!(["d2"]), "{r}");
}

#[test]
fn when_everything_is_modified_nothing_is_closed_but_the_result_says_so() {
    let mut c = Client::start(&[]);
    for i in 1..=33 {
        c.ok("doc_new", json!({ "width": 2, "height": 2 }));
        c.ok("draw_pixels", json!({ "doc": format!("d{i}"), "points": [[0, 0]], "color": "#f00" }));
    }
    let r = c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    // Only d34 is unmodified and it is the new one.
    assert!(r.get("closed_headless").is_none(), "{r}");
    assert!(r["note"].as_str().unwrap().contains("34 headless documents"), "{r}");
    assert_eq!(doc_ids(&mut c).len(), 34);
}

fn rgba(px: &[[u8; 4]]) -> Vec<u8> {
    px.iter().flatten().copied().collect()
}

#[test]
fn layer_import_png_replace_swaps_the_pixels_in_one_undo_step() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    c.ok("layer", json!({ "action": "set", "name": "body", "opacity": 128, "blend_mode": "multiply" }));
    c.ok("layer", json!({ "action": "add", "name": "top" }));
    c.ok("draw_pixels", json!({ "layer": "body", "points": [[0, 0], [3, 3]], "color": "#00ff00" }));
    let red = [255, 0, 0, 255];
    let blue = [0, 0, 255, 255];
    write_png(&c.path("new.png"), 2, 2, &rgba(&[red, blue, blue, red]));
    let hex = |c: &mut Client, x: u32, y: u32| {
        c.ok("get_pixels", json!({ "layer": 0, "format": "hex", "region": { "x": x, "y": y, "w": 1, "h": 1 } }))["rows"]
            [0][0]
            .as_str()
            .unwrap()
            .to_string()
    };
    let base = undo_steps(&mut c);
    let r = c.ok("layer", json!({ "action": "import_png", "layer": "body", "path": "new.png", "replace": true }));
    assert_eq!(undo_steps(&mut c), base + 1, "one undo step");
    assert_eq!(r["layers"].as_array().unwrap().len(), 2, "no layer was added: {r}");
    assert_eq!(r["layer"], 0);
    // Layer properties are kept, the old pixels are gone, the PNG sits at the top-left.
    let info = c.ok("doc_info", json!({ "sections": ["layers"] }));
    assert_eq!(info["layers"][0]["name"], "body");
    assert_eq!(info["layers"][0]["opacity"], 128);
    assert_eq!(info["layers"][0]["blend_mode"], "multiply");
    assert_eq!(info["layers"][1]["name"], "top");
    assert_eq!(hex(&mut c, 0, 0), "#ff0000ff");
    assert_eq!(hex(&mut c, 1, 0), "#0000ffff");
    assert_eq!(hex(&mut c, 3, 3), "#00000000", "the old pixel outside the PNG is gone");
    // One undo brings the old cel back.
    c.ok("undo", json!({}));
    assert_eq!(hex(&mut c, 0, 0), "#00ff00ff");
    assert_eq!(hex(&mut c, 3, 3), "#00ff00ff");

    // An offset, partly outside the canvas (clipped), and a rename.
    c.ok(
        "layer",
        json!({ "action": "import_png", "layer": 0, "path": "new.png", "replace": true, "x": 3, "y": -1, "name": "skin" }),
    );
    assert_eq!(hex(&mut c, 3, 0), "#0000ffff", "the PNG's bottom-left pixel lands at (3, 0)");
    assert_eq!(hex(&mut c, 2, 0), "#00000000");
    assert_eq!(hex(&mut c, 3, 1), "#00000000");
    assert_eq!(c.ok("doc_info", json!({ "sections": ["layers"] }))["layers"][0]["name"], "skin");

    // The target frame is chosen with `frame`; the other frame keeps its image.
    c.ok("frame", json!({ "action": "add" }));
    c.ok("layer", json!({ "action": "import_png", "layer": 0, "frame": 1, "path": "new.png", "replace": true }));
    let f1 = c.ok(
        "get_pixels",
        json!({ "layer": 0, "frame": 1, "format": "hex", "region": { "x": 0, "y": 0, "w": 1, "h": 1 } }),
    );
    assert_eq!(f1["rows"][0][0], "#ff0000ff");
    let f0 = c.ok(
        "get_pixels",
        json!({ "layer": 0, "frame": 0, "format": "hex", "region": { "x": 0, "y": 0, "w": 1, "h": 1 } }),
    );
    assert_eq!(f0["rows"][0][0], "#00000000");

    // Errors change nothing.
    let n = undo_steps(&mut c);
    c.ok("layer", json!({ "action": "add_group", "name": "grp" }));
    let n = n + 1;
    let e = c.err("layer", json!({ "action": "import_png", "layer": "grp", "path": "new.png", "replace": true }));
    assert!(e.contains("group"), "{e}");
    let e = c.err("layer", json!({ "action": "import_png", "layer": 0, "path": "new.png", "x": 1 }));
    assert!(e.contains("replace=true"), "{e}");
    let e = c.err("layer", json!({ "action": "set", "layer": 0, "replace": true }));
    assert!(e.contains("import_png"), "{e}");
    let e = c.err("layer", json!({ "action": "import_png", "layer": 0, "path": "missing.png", "replace": true }));
    assert!(e.contains("missing.png"), "{e}");
    assert_eq!(undo_steps(&mut c), n);
}

#[test]
fn replacing_a_linked_cel_leaves_the_other_frame_alone() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#00ff00" }));
    c.ok("frame", json!({ "action": "add" }));
    c.ok("cel", json!({ "action": "link", "layer": 0, "from_frame": 0, "to_frame": 1 }));
    write_png(&c.path("p.png"), 1, 1, &[255, 0, 0, 255]);
    c.ok("layer", json!({ "action": "import_png", "layer": 0, "frame": 1, "path": "p.png", "replace": true }));
    let px = |c: &mut Client, f: u32| {
        c.ok(
            "get_pixels",
            json!({ "layer": 0, "frame": f, "format": "hex", "region": { "x": 0, "y": 0, "w": 1, "h": 1 } }),
        )["rows"][0][0]
            .clone()
    };
    assert_eq!(px(&mut c, 0), "#00ff00ff", "frame 0 still has its own image");
    assert_eq!(px(&mut c, 1), "#ff0000ff");
}

#[test]
fn delete_is_accepted_where_remove_exists() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    c.ok("layer", json!({ "action": "add", "name": "a" }));
    let r = c.ok("layer", json!({ "action": "delete", "layer": "a" }));
    assert_eq!(r["layers"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(c.ok("layer", json!({ "action": "add" }))["layers"].as_array().unwrap().len(), 2);
    c.ok("layer", json!({ "action": "remove", "layer": 1 }));

    c.ok("frame", json!({ "action": "add", "count": 2 }));
    assert_eq!(c.ok("frame", json!({ "action": "delete", "frame": 2 }))["frames"], 2);

    c.ok("tag", json!({ "action": "add", "name": "t", "from": 0, "to": 1 }));
    assert_eq!(c.ok("tag", json!({ "action": "delete", "tag": "t" }))["tags"], json!([]));

    c.ok("slice", json!({ "action": "add", "name": "s", "bounds": { "x": 0, "y": 0, "w": 2, "h": 2 } }));
    c.ok("slice", json!({ "action": "delete", "slice": "s" }));
    assert_eq!(c.ok("doc_info", json!({ "sections": ["slices"] }))["slices"], json!([]));

    c.ok("palette", json!({ "action": "add", "colors": ["#123456", "#abcdef"] }));
    let n = c.ok("palette", json!({ "action": "get" }))["palette"].as_array().unwrap().len();
    let after = c.ok("palette", json!({ "action": "delete", "indices": [0] }));
    assert_eq!(after["palette"].as_array().unwrap().len(), n - 1, "{after}");

    c.ok("draw_pixels", json!({ "layer": 0, "frame": 0, "points": [[0, 0]], "color": "#f00" }));
    c.ok("cel", json!({ "action": "remove", "layer": 0, "frame": 0 }));
    assert_eq!(c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 0 }))["color"], "#00000000");
    c.ok("draw_pixels", json!({ "layer": 0, "frame": 0, "points": [[0, 0]], "color": "#f00" }));
    c.ok("cel", json!({ "action": "delete", "layer": 0, "frame": 0 }));
    assert_eq!(c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 0 }))["color"], "#00000000");

    // Tween, tileset and terrain spellings parse; they fail later, for lack of keys, tilesets or a tilemap layer.
    let parsed =
        |e: &str| !e.contains("unknown variant") && !e.contains("invalid arguments") && !e.contains("unknown arg");
    let e = c.err("tween", json!({ "action": "delete_key", "property": "x", "frame": 0 }));
    assert!(parsed(&e) && e.contains("no key"), "{e}");
    let e = c.err("tileset", json!({ "action": "remove_tile", "tile": 1 }));
    assert!(parsed(&e), "{e}");
    let e = c.err("terrain", json!({ "action": "delete" }));
    assert!(parsed(&e) && e.contains("tilemap"), "{e}");
}

#[test]
fn unknown_actions_list_the_valid_ones() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let e = c.err("layer", json!({ "action": "explode" }));
    assert!(
        e.contains("explode") && e.contains("add_group") && e.contains("merge_down") && e.contains("remove"),
        "{e}"
    );
    let e = c.err("slice", json!({ "action": "destroy" }));
    assert!(e.contains("destroy") && e.contains("list") && e.contains("remove"), "{e}");
    // Aliases are not advertised as extra enum values; the schema keeps its plain list.
    let tools = c.request("tools/list", json!({}));
    let layer = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "layer").unwrap();
    let actions = layer["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap();
    assert!(actions.contains(&json!("remove")) && !actions.contains(&json!("delete")), "{actions:?}");
    assert!(layer["description"].as_str().unwrap().contains("alias: delete"));
}

#[test]
fn help_has_a_topic_for_every_tool_group() {
    let mut c = Client::start(&[]);
    let slices = c.ok("help", json!({ "topic": "slices" }));
    let text = slices.as_str().unwrap();
    assert!(text.contains("9-patch") && text.contains("slice {action}"), "{text}");
    for topic in [
        "documents",
        "layers",
        "drawing",
        "selection",
        "transform",
        "filters",
        "tilemaps",
        "slices",
        "tween",
        "export",
        "history",
        "live",
        "palettes",
        "tools",
        "guide",
    ] {
        let t = c.ok("help", json!({ "topic": topic }));
        assert!(t.as_str().is_some_and(|s| s.len() > 80), "{topic}: {t}");
    }
    let e = c.err("help", json!({ "topic": "nonsense" }));
    assert!(e.contains("slices") && e.contains("tilemaps"), "{e}");
}

#[test]
fn run_lua_accepts_script_and_code_as_aliases_for_source() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    assert_eq!(c.ok("run_lua", json!({ "script": "print('a')" }))["output"], "a");
    assert_eq!(c.ok("run_lua", json!({ "code": "print('b')" }))["output"], "b");
    assert_eq!(c.ok("run_lua", json!({ "source": "print('c')" }))["output"], "c");
    let e = c.err("run_lua", json!({ "script": "print(1)", "code": "print(2)" }));
    assert!(e.contains("aliases"), "{e}");
    let e = c.err("run_lua", json!({ "script": "print(1)", "path": "x.lua" }));
    assert!(e.contains("either"), "{e}");
    let e = c.err("run_lua", json!({}));
    assert!(e.contains("script") && e.contains("code"), "{e}");
}

#[test]
fn run_lua_sources_up_to_16_mib_are_accepted() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    // 2 MiB (the old limit was 1 MiB) of comments, then a statement.
    let big = format!("--{}\nprint('big')", "x".repeat(2 << 20));
    assert_eq!(c.ok("run_lua", json!({ "source": big }))["output"], "big");
    std::fs::write(c.path("huge.lua"), "-".repeat((16 << 20) + 1)).unwrap();
    let e = c.err("run_lua", json!({ "path": "huge.lua" }));
    assert!(e.contains("16 MiB"), "{e}");
}

#[test]
fn run_lua_timeout_seconds_raises_the_limits() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    // The instruction budget is proportional to the time: 30 M instructions per second.
    let work = "for i = 1, 40000000 do end print('done')";
    let e = c.err("run_lua", json!({ "source": work, "timeout_seconds": 1 }));
    assert!(e.contains("limit exceeded") && e.contains("document unchanged"), "{e}");
    let r = c.ok("run_lua", json!({ "source": work, "timeout_seconds": 4 }));
    assert_eq!(r["output"], "done");
    // A runaway script still stops.
    let t = Instant::now();
    let e = c.err("run_lua", json!({ "source": "while true do end", "timeout_seconds": 1 }));
    assert!(e.contains("limit exceeded"), "{e}");
    assert!(t.elapsed().as_secs() < 10);
    for bad in [0, 121] {
        let e = c.err("run_lua", json!({ "source": "print(1)", "timeout_seconds": bad }));
        assert!(e.contains("timeout_seconds must be 1-120"), "{e}");
    }
    // The schema documents the argument.
    let tools = c.request("tools/list", json!({}));
    let lua = tools["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == "run_lua").unwrap();
    let props = lua["inputSchema"]["properties"].as_object().unwrap();
    for k in ["source", "script", "code", "path", "timeout_seconds"] {
        assert!(props.contains_key(k), "{k}");
    }
    assert!(lua["description"].as_str().unwrap().contains("timeout_seconds"));
}

#[test]
fn run_lua_reaches_files_only_inside_allow_paths() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let work = c.path("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("in.txt"), "hello").unwrap();
    let read = format!(
        "local f = assert(io.open({:?}, 'r')) print(f:read('a')) f:close()",
        work.join("in.txt").display().to_string()
    );
    // Without allow_paths nobody can approve the access, so it is denied and the document is unchanged.
    let e = c.err("run_lua", json!({ "source": read }));
    assert!(e.contains("script failed"), "{e}");
    // Inside allow_paths: read and write work.
    let r = c.ok("run_lua", json!({ "source": read, "allow_paths": [work.display().to_string()] }));
    assert_eq!(r["output"], "hello", "{r}");
    let write = format!(
        "local f = assert(io.open({:?}, 'w')) f:write('out') f:close()",
        work.join("out.txt").display().to_string()
    );
    c.ok("run_lua", json!({ "source": write, "allow_paths": [work.display().to_string()] }));
    assert_eq!(std::fs::read_to_string(work.join("out.txt")).unwrap(), "out");
    // A sibling folder stays closed.
    let other = c.path("other");
    std::fs::create_dir_all(&other).unwrap();
    let outside = format!("assert(io.open({:?}, 'w'))", other.join("x.txt").display().to_string());
    c.err("run_lua", json!({ "source": outside, "allow_paths": [work.display().to_string()] }));
    assert!(!other.join("x.txt").exists());
}

#[test]
fn run_lua_documents_made_by_the_script_become_headless_documents() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let r = c.ok("run_lua", json!({ "source": "local s = Sprite(16, 12) app.activeSprite = s" }));
    let created = r["new_documents"].as_array().expect("new_documents");
    assert_eq!(created.len(), 1, "{r}");
    let id = created[0].as_str().unwrap();
    let info = c.ok("doc_info", json!({ "doc": id, "sections": [] }));
    assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(16), Some(12)), "{info}");
}
