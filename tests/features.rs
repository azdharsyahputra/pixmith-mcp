//! Every tool group, end to end through the real binary over stdio.

mod common;
use common::{Client, png_size};
use serde_json::{Value, json};

fn undo_steps(c: &mut Client) -> u64 {
    c.ok("history", json!({}))["undo_steps"].as_u64().unwrap()
}

#[test]
fn draw_render_and_undo() {
    let mut c = Client::start(&[]);
    let d = c.ok("doc_new", json!({ "width": 16, "height": 16, "name": "hero" }));
    assert_eq!(d["doc"], "d1");
    assert_eq!(d["dirty"], false);
    let base = undo_steps(&mut c);

    let r = c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [2, 2], "to": [5, 5], "color": "#ff0000", "filled": true }),
    );
    assert_eq!(r["pixels_changed"], 16);
    assert_eq!(c.pixel(2, 2), "#ff0000ff");
    assert_eq!(c.pixel(5, 5), "#ff0000ff");
    assert_eq!(c.pixel(6, 6), "#00000000");
    assert_eq!(undo_steps(&mut c), base + 1, "one call, one undo step");

    c.ok("draw_shape", json!({ "shape": "line", "from": [0, 15], "to": [15, 15], "color": "blue", "brush_size": 1 }));
    assert_eq!(c.pixel(8, 15), "#0000ffff");
    c.ok("draw_shape", json!({ "shape": "ellipse", "from": [8, 0], "to": [15, 7], "color": "#0f0" }));
    c.ok(
        "draw_pixels",
        json!({ "pixels": [{ "x": 0, "y": 0, "color": "#ffffff" }, { "x": 1, "y": 0, "color": "#000000" }] }),
    );
    assert_eq!(c.pixel(0, 0), "#ffffffff");
    c.ok("draw_pixels", json!({ "points": [[0, 1], [0, 2]], "color": "#ffff00", "mirror": "horizontal" }));
    assert_eq!(c.pixel(15, 1), "#ffff00ff", "mirrored across the center");
    c.ok("draw_stroke", json!({ "points": [[0, 8], [3, 11], [6, 8]], "color": "#808080", "pixel_perfect": true }));
    c.ok("draw_polygon", json!({ "points": [[9, 9], [14, 9], [12, 13]], "color": "#ff00ff", "filled": true }));
    assert_eq!(c.pixel(12, 10), "#ff00ffff");
    c.ok("draw_curve", json!({ "points": [[0, 13], [4, 9], [8, 17], [12, 13]], "color": "#00ffff" }));
    let t = c.ok("draw_text", json!({ "text": "Hi", "x": 1, "y": 6, "color": "#ffffff", "measure_only": true }));
    assert!(t["width"].as_i64().unwrap() > 5 && t["height"] == 8);
    c.ok("draw_text", json!({ "text": "A", "x": 9, "y": 1, "color": "#ffffff" }));
    c.ok("spray", json!({ "points": [[12, 4]], "color": "#123456", "radius": 2, "density": 50, "seed": 7 }));
    let none = c.ok("draw_pixels", json!({ "points": [[40, 40]], "color": "#fff" }));
    assert_eq!(none["pixels_changed"], 0);

    // Ink modes and brushes.
    c.ok("layer", json!({ "action": "add", "name": "ink" }));
    c.ok("draw_shape", json!({ "shape": "rectangle", "from": [0, 0], "to": [3, 3], "color": "#ff000080", "filled": true, "ink": "alpha" }));
    let p = c.ok("get_pixel", json!({ "x": 0, "y": 0 }));
    assert_ne!(p["color"], "#ffffffff", "alpha ink blended over white: {p}");
    c.ok("draw_shape", json!({ "shape": "line", "from": [6, 12], "to": [6, 12], "color": "#000", "brush_size": 3, "brush_shape": "circle" }));

    // Fill: exact, global, tolerance, pattern.
    c.ok("layer", json!({ "action": "add", "name": "fills" }));
    c.ok("fill", json!({ "x": 0, "y": 0, "color": "#202020" }));
    assert_eq!(
        c.ok("get_pixels", json!({ "layer": "fills", "region": { "x": 0, "y": 0, "w": 2, "h": 1 } }))["rows"][0],
        "aa"
    );
    c.ok("draw_pixels", json!({ "points": [[3, 3]], "color": "#252525" }));
    let r = c.ok("fill", json!({ "x": 0, "y": 0, "color": "#ff8800", "tolerance": 8 }));
    assert_eq!(r["pixels_changed"], 256, "tolerance takes the near color too");
    c.ok(
        "fill",
        json!({ "x": 0, "y": 0, "color": "#000000", "contiguous": false, "pattern": "checker", "color2": "#ffffff" }),
    );
    let px = c.ok("get_pixels", json!({ "layer": "fills", "region": { "x": 0, "y": 0, "w": 4, "h": 1 } }));
    assert_eq!(px["rows"][0], "aabb", "checker of 2x2 squares: {px}");
    c.ok(
        "gradient",
        json!({ "from": [0, 0], "to": [15, 0], "color_from": "#000000", "color_to": "#ffffff", "dither": 4 }),
    );

    // Render: a real PNG, auto scaled, plus isolation, region, onion skin.
    let r = c.call("render", json!({}));
    let imgs = Client::images(&r);
    assert_eq!(imgs.len(), 1);
    assert_eq!((imgs[0].0, imgs[0].1), (512, 512));
    let r = c.call(
        "render",
        json!({ "layer": 0, "scale": 2, "region": { "x": 2, "y": 2, "w": 4, "h": 4 }, "background": "transparent" }),
    );
    let (w, h, px) = &Client::images(&r)[0];
    assert_eq!((*w, *h), (8, 8));
    assert_eq!(&px[..4], &[255, 0, 0, 255], "isolated layer 0 shows the red square");
    c.ok("frame", json!({ "action": "duplicate" }));
    let r = c.call("render", json!({ "frames": [0, 1], "onion_skin": { "before": 1 } }));
    assert_eq!(Client::images(&r).len(), 2);

    // get_pixels formats and limits.
    let hexes = c.ok(
        "get_pixels",
        json!({ "layer": 0, "frame": 0, "format": "hex", "region": { "x": 2, "y": 2, "w": 2, "h": 1 } }),
    );
    assert_eq!(hexes["rows"], json!([["#ff0000ff", "#ff0000ff"]]));
    assert_eq!(c.ok("get_pixels", json!({ "format": "hex" }))["rows"].as_array().unwrap().len(), 16);
    c.ok("doc_new", json!({ "width": 200, "height": 200 }));
    let e = c.err("get_pixels", json!({ "doc": "d2" }));
    assert!(e.contains("smaller region"), "{e}");
    c.ok("doc_close", json!({ "doc": "d2" }));

    // Undo / redo / history / goto.
    let before = c.pixel(12, 10);
    let n = undo_steps(&mut c);
    let u = c.ok("undo", json!({ "steps": 1 }));
    assert_eq!(u["undone"], 1);
    assert_eq!(undo_steps(&mut c), n - 1);
    c.ok("redo", json!({}));
    assert_eq!(c.pixel(12, 10), before);
    let hist = c.ok("history", json!({ "limit": 500 }));
    let labels: Vec<&str> = hist["steps"].as_array().unwrap().iter().map(|s| s["label"].as_str().unwrap()).collect();
    assert!(labels.contains(&"Rectangle") && labels.contains(&"Polygon") && labels.contains(&"Fill"), "{labels:?}");
    c.ok("history_goto", json!({ "position": base }));
    assert_eq!(c.pixel(2, 2), "#00000000", "back to the start");
    c.ok("history_goto", json!({ "position": n }));
    let px = c.ok(
        "get_pixels",
        json!({ "layer": 0, "frame": 0, "format": "hex", "region": { "x": 2, "y": 2, "w": 1, "h": 1 } }),
    );
    assert_eq!(px["rows"], json!([["#ff0000ff"]]));
}

#[test]
fn transactions_make_one_undo_step() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let n = undo_steps(&mut c);
    c.ok("transaction", json!({ "action": "begin", "label": "Draw face" }));
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#f00" }));
    c.ok("draw_pixels", json!({ "points": [[2, 2]], "color": "#0f0" }));
    c.ok("layer", json!({ "action": "add" }));
    c.ok("draw_pixels", json!({ "points": [[3, 3]], "color": "#00f" }));
    let e = c.err("undo", json!({}));
    assert!(e.contains("transaction"), "{e}");
    c.ok("transaction", json!({ "action": "end" }));
    assert_eq!(undo_steps(&mut c), n + 1);
    let h = c.ok("history", json!({}));
    assert_eq!(h["next_undo"], "Draw face");
    c.ok("undo", json!({}));
    assert_eq!(c.pixel(1, 1), "#00000000");
    assert_eq!(c.pixel(3, 3), "#00000000");
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["layers"], 1);
}

#[test]
fn selection_clipboard_transform() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 16, "height": 16 }));
    c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [0, 0], "to": [3, 1], "color": "#ff0000", "filled": true }),
    );
    let s = c.ok("select", json!({ "action": "rect", "x": 0, "y": 0, "w": 4, "h": 2 }));
    assert_eq!(s["bounds"], json!({ "x": 0, "y": 0, "w": 4, "h": 2 }));
    c.ok("select", json!({ "action": "rect", "x": 0, "y": 0, "w": 1, "h": 1, "mode": "subtract" }));
    c.ok("select", json!({ "action": "all" }));
    c.ok("select", json!({ "action": "invert" }));
    assert_eq!(c.ok("doc_info", json!({ "sections": ["selection"] }))["selection"]["active"], false);
    c.ok("select", json!({ "action": "wand", "x": 1, "y": 1 }));
    let s = c.ok("select", json!({ "action": "expand", "amount": 1 }));
    assert_eq!(s["bounds"]["w"], 5);
    c.ok("select", json!({ "action": "contract", "amount": 1 }));
    c.ok("select", json!({ "action": "polygon", "points": [[0, 0], [3, 0], [3, 1], [0, 1]] }));
    c.ok("select", json!({ "action": "by_color", "x": 0, "y": 0, "tolerance": 10 }));

    // Copy, paste floating, move, commit.
    let cp = c.ok("clipboard", json!({ "action": "copy" }));
    assert_eq!(cp["copied"]["w"], 4);
    c.ok("clipboard", json!({ "action": "paste", "x": 8, "y": 8 }));
    assert_eq!(c.ok("doc_info", json!({ "sections": ["selection"] }))["selection"]["floating_paste"], true);
    c.ok("clipboard", json!({ "action": "move", "dx": 2, "dy": 0 }));
    c.ok("clipboard", json!({ "action": "commit" }));
    assert_eq!(c.pixel(10, 8), "#ff0000ff");
    assert_eq!(c.pixel(8, 8), "#00000000");
    c.ok("select", json!({ "action": "none" }));

    // Cut and the selected-pixel edits.
    c.ok("select", json!({ "action": "rect", "x": 10, "y": 8, "w": 4, "h": 2 }));
    c.ok("select", json!({ "action": "move_pixels", "dx": 0, "dy": 4 }));
    assert_eq!(c.pixel(10, 12), "#ff0000ff");
    c.ok("select", json!({ "action": "delete" }));
    assert_eq!(c.pixel(10, 12), "#00000000");
    c.ok("select", json!({ "action": "none" }));

    // A PNG from disk as a floating paste.
    let png = c.p("stamp.png");
    c.ok("doc_new", json!({ "width": 3, "height": 3, "background": "#00ff00" }));
    c.ok("export", json!({ "doc": "d2", "format": "png", "path": png }));
    c.ok("clipboard", json!({ "doc": "d1", "action": "paste_image", "path": png, "x": 0, "y": 13 }));
    c.ok("clipboard", json!({ "doc": "d1", "action": "commit" }));
    c.ok("doc_close", json!({ "doc": "d2" }));
    assert_eq!(c.pixel(1, 14), "#00ff00ff");

    // Flips and 90-degree turns of the cel, all frames (the committed paste is still selected).
    assert_eq!(c.ok("doc_info", json!({ "sections": ["selection"] }))["selection"]["active"], true);
    c.ok("select", json!({ "action": "none" }));
    c.ok("frame", json!({ "action": "duplicate", "frame": 0 }));
    c.ok("transform", json!({ "action": "flip_horizontal", "all_frames": true }));
    assert_eq!(c.ok("get_pixel", json!({ "x": 15, "y": 0, "frame": 1 }))["color"], "#ff0000ff");
    c.ok("transform", json!({ "action": "rotate_180" }));
    c.ok("transform", json!({ "action": "rotate_cw" }));
    c.ok("transform", json!({ "action": "rotate", "angle": 45, "rotsprite": true }));

    // Free transform session.
    c.ok("doc_new", json!({ "width": 16, "height": 16 }));
    c.ok(
        "draw_shape",
        json!({ "doc": "d3", "shape": "rectangle", "from": [4, 4], "to": [7, 7], "color": "#fff", "filled": true }),
    );
    c.ok("select", json!({ "doc": "d3", "action": "rect", "x": 4, "y": 4, "w": 4, "h": 4 }));
    let b = c.ok("free_transform", json!({ "doc": "d3", "action": "begin" }));
    assert_eq!(b["active"], true);
    assert_eq!(b["width"], 4.0);
    let s =
        c.ok("free_transform", json!({ "doc": "d3", "action": "set", "width": 8, "height": 8, "anchor": "top_left" }));
    assert_eq!(s["width"], 8.0);
    c.ok("free_transform", json!({ "doc": "d3", "action": "commit" }));
    assert_eq!(c.ok("get_pixel", json!({ "doc": "d3", "x": 11, "y": 11 }))["color"], "#ffffffff");
    c.ok("undo", json!({ "doc": "d3" }));
    assert_eq!(c.ok("get_pixel", json!({ "doc": "d3", "x": 11, "y": 11 }))["color"], "#00000000");

    // Sprite and canvas size.
    let s = c.ok("sprite_size", json!({ "doc": "d3", "action": "scale", "factor": 2 }));
    assert_eq!((s["width"].as_u64(), s["height"].as_u64()), (Some(32), Some(32)));
    let s =
        c.ok("sprite_size", json!({ "doc": "d3", "action": "canvas", "width": 40, "height": 32, "anchor": "left" }));
    assert_eq!(s["width"], 40);
    c.ok("select", json!({ "doc": "d3", "action": "rect", "x": 0, "y": 0, "w": 10, "h": 10 }));
    let s = c.ok("select", json!({ "doc": "d3", "action": "crop" }));
    assert_eq!(s["canvas"], json!([10, 10]));
}

#[test]
fn layers_frames_cels_tags() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let r = c.ok("layer", json!({ "action": "add", "name": "body" }));
    assert_eq!(r["layer"], 1);
    assert_eq!(r["active_layer"], 1);
    c.ok(
        "layer",
        json!({ "action": "set", "layer": "body", "opacity": 128, "blend_mode": "multiply", "visible": true }),
    );
    let info = c.ok("doc_info", json!({ "sections": ["layers"] }));
    assert_eq!(info["layers"][1]["blend_mode"], "multiply");
    assert_eq!(info["layers"][1]["opacity"], 128);
    c.ok("layer", json!({ "action": "duplicate", "layer": "body" }));
    c.ok("layer", json!({ "action": "add_group", "name": "char" }));
    let g = c.ok("layer", json!({ "action": "into_group", "layer": "body", "group": "char" }));
    let layers = g["layers"].as_array().unwrap();
    let body = layers.iter().find(|l| l["name"] == "body").unwrap();
    assert_eq!(body["level"], 1, "{g}");
    c.ok("layer", json!({ "action": "out_of_group", "layer": "body" }));
    c.ok("layer", json!({ "action": "wrap_in_group", "layer": "body" }));
    let e = c.err("layer", json!({ "action": "move_down", "layer": 0 }));
    assert!(e.contains("cannot move"), "{e}");
    c.ok("layer", json!({ "action": "move_to", "layer": 0, "to": 1 }));
    let names: Vec<String> = c.ok("doc_info", json!({ "sections": ["layers"] }))["layers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap().to_string())
        .collect();
    let copy = names.iter().position(|n| n.contains("copy")).expect("duplicated layer");
    c.ok("draw_pixels", json!({ "layer": copy, "points": [[0, 0]], "color": "#f00" }));
    c.ok("layer", json!({ "action": "set", "layer": copy, "name": "top" }));
    c.ok("layer", json!({ "action": "remove", "layer": "top" }));
    // merge down: two image layers next to each other.
    c.ok("layer", json!({ "action": "add", "layer": 0, "name": "a" }));
    c.ok("draw_pixels", json!({ "layer": "a", "points": [[1, 1]], "color": "#0f0" }));
    c.ok("layer", json!({ "action": "add", "layer": "a", "name": "b" }));
    c.ok("draw_pixels", json!({ "layer": "b", "points": [[2, 2]], "color": "#00f" }));
    c.ok("layer", json!({ "action": "merge_down", "layer": "b" }));
    let px = c.ok("get_pixels", json!({ "layer": "a", "region": { "x": 1, "y": 1, "w": 2, "h": 2 } }));
    assert_eq!(px["rows"], json!(["a.", ".b"]), "{px}");

    // Frames.
    let f = c.ok("frame", json!({ "action": "add", "count": 3 }));
    assert_eq!(f["frames"], 4);
    c.ok("frame", json!({ "action": "set_duration", "all": true, "duration_ms": 80 }));
    c.ok("frame", json!({ "action": "set_duration", "frames": [1, 2], "duration_ms": 120 }));
    let f = c.ok("frame", json!({ "action": "duplicate", "frame": 0 }));
    assert_eq!(f["frames"], 5);
    assert_eq!(f["durations_ms"], json!([80, 80, 120, 120, 80]));
    c.ok("frame", json!({ "action": "move", "frame": 4, "to": 0 }));
    c.ok("frame", json!({ "action": "remove", "frame": 4 }));

    // Tags.
    let t = c.ok(
        "tag",
        json!({ "action": "add", "name": "walk", "from": 0, "to": 2, "direction": "ping_pong", "color": "#ff8800" }),
    );
    assert_eq!(t["tags"][0]["direction"], "ping_pong");
    assert_eq!(t["tags"][0]["color"], "#ff8800");
    let t = c.ok("tag", json!({ "action": "update", "tag": "walk", "to": 3, "repeat": 2, "name": "run" }));
    assert_eq!(t["tags"][0]["name"], "run");
    assert_eq!(t["tags"][0]["to"], 3);
    c.ok("tag", json!({ "action": "add", "name": "idle", "from": 3 }));
    let t = c.ok("tag", json!({ "action": "remove", "tag": "idle" }));
    assert_eq!(t["tags"].as_array().unwrap().len(), 1);

    // Cels.
    let a = "a";
    let e = c.err("cel", json!({ "action": "link", "layer": a, "from_frame": 0, "to_frame": 2 }));
    assert!(e.contains("no image"), "frame 0 is the empty one moved to the front: {e}");
    c.ok("cel", json!({ "action": "link", "layer": a, "from_frame": 1, "to_frame": 3 }));
    let info = c.ok("cel", json!({ "action": "info", "layer": a, "frame": 2 }));
    assert_eq!(info["linked"], true);
    c.ok("cel", json!({ "action": "unlink", "layer": a, "frame": 2 }));
    c.ok("cel", json!({ "action": "set_opacity", "layer": a, "frame": 1, "opacity": 200 }));
    assert_eq!(c.ok("cel", json!({ "action": "info", "layer": a, "frame": 1 }))["opacity"], 200);
    c.ok("cel", json!({ "action": "copy", "layer": a, "frame": 1 }));
    c.ok("cel", json!({ "action": "paste", "layer": a, "frame": 3 }));
    c.ok("cel", json!({ "action": "move", "layer": a, "frame": 3, "df": -1, "copy": true }));
    c.ok("cel", json!({ "action": "clear", "layer": a, "frame": 3 }));
    let cels = c.ok("doc_info", json!({ "sections": ["cels"] }));
    assert!(
        cels["cels"]["rows"].as_array().unwrap().iter().any(|r| r["frames"].as_str().unwrap().contains('#')),
        "{cels}"
    );
    c.ok("doc_set_active", json!({ "layer": "a", "frame": 2 }));
    assert_eq!(c.ok("doc_list", json!({}))["documents"][0]["active_frame"], 2);
}

#[test]
fn palette_color_mode_filters() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8, "palette_preset": "PICO-8" }));
    let p = c.ok("palette", json!({ "action": "get" }));
    assert_eq!(p["size"], 16);
    let presets = c.ok("palette", json!({ "action": "list_presets" }));
    assert!(presets["presets"].as_array().unwrap().len() >= 5);
    c.ok("palette", json!({ "action": "add", "colors": ["#123456", "#abcdef"] }));
    c.ok("palette", json!({ "action": "set", "index": 0, "color": "#010203" }));
    c.ok("palette", json!({ "action": "insert", "index": 1, "color": "#ff0000" }));
    c.ok("palette", json!({ "action": "move", "indices": [1], "to": 0 }));
    let p = c.ok("palette", json!({ "action": "duplicate", "indices": [0] }));
    assert_eq!(p["palette"][0], "#ff0000ff");
    assert_eq!(p["palette"][1], "#ff0000ff");
    c.ok("palette", json!({ "action": "remove", "indices": [1] }));
    c.ok("palette", json!({ "action": "ramp", "index": 0, "to": 1, "steps": 3 }));
    c.ok("palette", json!({ "action": "sort", "by": "luminance" }));
    let gpl = c.p("pal/test.gpl");
    let s = c.ok("palette", json!({ "action": "save_file", "path": gpl }));
    assert!(std::path::Path::new(&gpl).is_file(), "{s}");
    c.ok("palette", json!({ "action": "replace_all", "colors": ["#000000", "#ffffff", "#ff0000", "#00ff00"] }));
    assert_eq!(c.ok("palette", json!({ "action": "get" }))["size"], 4);
    c.ok("palette", json!({ "action": "load_file", "path": gpl }));
    assert!(c.ok("palette", json!({ "action": "get" }))["size"].as_u64().unwrap() > 10);
    c.ok("palette", json!({ "action": "load_preset", "name": "game boy (4)" }));

    // Color modes.
    c.ok("gradient", json!({ "from": [0, 0], "to": [7, 0], "color_from": "#000000", "color_to": "#ffffff" }));
    let m = c.ok("color_mode", json!({ "mode": "indexed", "dither": 2 }));
    assert_eq!(m["color_mode"], "indexed");
    let p = c.ok("get_pixel", json!({ "x": 4, "y": 0 }));
    assert!(p["palette_index"].is_u64(), "indexed pixels use palette colors: {p}");
    c.ok("color_mode", json!({ "mode": "grayscale" }));
    c.ok("color_mode", json!({ "mode": "rgba" }));
    c.ok("color_mode", json!({ "conform_only": true }));
    c.ok("palette", json!({ "action": "from_sprite", "max_colors": 8 }));

    // Filters on a fresh sprite.
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let d = json!("d2");
    c.ok(
        "draw_shape",
        json!({ "doc": d, "shape": "rectangle", "from": [2, 2], "to": [5, 5], "color": "#ff0000", "filled": true }),
    );
    let r = c.ok("filter", json!({ "doc": d, "kind": "invert" }));
    assert_eq!(r["changed"], true);
    assert_eq!(c.ok("get_pixel", json!({ "doc": d, "x": 2, "y": 2 }))["color"], "#00ffffff");
    c.ok("undo", json!({ "doc": d }));
    let pv = c.call("filter", json!({ "doc": d, "kind": "outline", "color": "#000000", "preview": true }));
    assert_eq!(Client::images(&pv).len(), 1);
    assert_eq!(c.ok("get_pixel", json!({ "doc": d, "x": 1, "y": 2 }))["color"], "#00000000", "preview changes nothing");
    let steps = c.ok("history", json!({ "doc": d }))["undo_steps"].clone();
    c.ok("filter", json!({ "doc": d, "kind": "outline", "color": "#000000" }));
    assert_eq!(c.ok("get_pixel", json!({ "doc": d, "x": 1, "y": 2 }))["color"], "#000000ff");
    for (kind, extra) in [
        ("replace_color", json!({ "from_color": "#ff0000", "to_color": "#00ff00" })),
        ("desaturate", json!({})),
        ("hsl", json!({ "hue": 30, "saturation": 10, "lightness": -5 })),
        ("brightness_contrast", json!({ "brightness": 10, "contrast": 20 })),
        ("color_balance", json!({ "red": 10, "green": -10, "blue": 5 })),
        ("drop_shadow", json!({ "dx": 1, "dy": 1, "color": "#00000080" })),
        ("remove_isolated_pixels", json!({ "eight_connected": true })),
        ("blur", json!({})),
        ("sharpen", json!({})),
        ("curves", json!({ "curve_rgb": [[0, 0], [128, 160], [255, 255]] })),
    ] {
        let mut args = json!({ "doc": d, "kind": kind });
        args.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        c.ok("filter", args);
    }
    c.ok(
        "filter",
        json!({ "doc": d, "kind": "invert", "region": { "x": 0, "y": 0, "w": 2, "h": 2 }, "target": "layer" }),
    );
    // A filter that changes nothing leaves no undo step.
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let n = c.ok("history", json!({ "doc": "d3" }))["undo_steps"].clone();
    let r = c.ok("filter", json!({ "doc": "d3", "kind": "blur" }));
    assert_eq!(r["changed"], false);
    assert_eq!(c.ok("history", json!({ "doc": "d3" }))["undo_steps"], n);
    let _ = steps;
}

#[test]
fn tilemaps_and_terrain() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 32, "height": 32 }));
    let t = c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 8, "name": "ground" }));
    let layer = t["layer"].clone();
    assert_eq!(t["tileset"], 0);
    let ts = c.ok("tileset", json!({ "action": "add_tile", "rows": ["aaaaaaaa", "abbbbbba", "abbbbbba", "abbbbbba", "abbbbbba", "abbbbbba", "abbbbbba", "aaaaaaaa"], "legend": { "a": "#000000", "b": "#00ff00" } }));
    assert_eq!(ts["tile"], 1);
    let tile = c.ok("tileset", json!({ "action": "get_tile", "tile": 1 }));
    assert_eq!(tile["rows"][1], "abbbbbba");
    c.ok("tilemap", json!({ "action": "stamp", "layer": layer, "x": 8, "y": 8, "tile": 1 }));
    assert_eq!(c.pixel(9, 9), "#00ff00ff");
    let cell = c.ok("tilemap", json!({ "action": "stamp", "x": 16, "y": 8, "tile": 1, "flip_x": true }));
    assert_eq!(cell["cell"]["flip_x"], true);
    let g = c.ok("tilemap", json!({ "action": "grid" }));
    assert_eq!(g["rows"].as_array().unwrap().len(), 4, "{g}");
    assert!(g["rows"][1].as_str().unwrap().contains('1'), "{g}");
    c.ok("tileset", json!({ "action": "duplicate_tile", "tile": 1 }));
    c.ok(
        "tileset",
        json!({ "action": "set_tile", "tile": 2, "rows": vec!["cccccccc"; 8], "legend": { "c": "#0000ff" } }),
    );
    c.ok("tileset", json!({ "action": "rename", "name": "ground tiles" }));
    c.ok("tileset", json!({ "action": "set_base_index", "base_index": 1 }));
    let img = c.call("tileset", json!({ "action": "image" }));
    assert_eq!(Client::images(&img).len(), 1);
    c.ok("tileset", json!({ "action": "delete_tile", "tile": 2 }));
    let list = c.ok("tileset", json!({ "action": "list" }));
    assert_eq!(list["tilesets"][0]["name"], "ground tiles");
    assert_eq!(list["tilesets"][0]["tiles"], 2);

    // Convert an image layer to a tilemap and back.
    c.ok("layer", json!({ "action": "add", "layer": 0, "name": "deco" }));
    c.ok("draw_shape", json!({ "layer": "deco", "shape": "rectangle", "from": [0, 0], "to": [7, 7], "color": "#ff0000", "filled": true }));
    c.ok("tilemap", json!({ "action": "convert", "layer": "deco", "tile_width": 8, "flip_aware": true }));
    c.ok("tilemap", json!({ "action": "to_image", "layer": "deco" }));

    // Terrain auto-tiling on a fresh tilemap layer.
    let t = c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 8, "name": "terrain" }));
    let tl = t["layer"].clone();
    let e = c.err("terrain_paint", json!({ "action": "rect", "terrain": 1, "x": 0, "y": 0, "w": 16, "h": 16 }));
    assert!(e.contains("no terrains"), "{e}");
    let r = c.ok(
        "terrain",
        json!({ "action": "create", "layer": tl, "mode": "sides", "name": "grass", "color": "#33aa33" }),
    );
    assert_eq!(r["mode"], "sides");
    let r = c.ok("terrain", json!({ "action": "generate_template" }));
    assert!(r["result"]["tiles_added"].as_u64().unwrap() >= 16, "{r}");
    let r = c.ok("terrain", json!({ "action": "assign_masks" }));
    assert!(r["result"]["tiles_assigned"].as_u64().unwrap() >= 16, "{r}");
    let r = c.ok("terrain", json!({ "action": "add", "name": "water", "color": "#3355ff" }));
    assert_eq!(r["terrains"].as_array().unwrap().len(), 2);
    c.ok("terrain", json!({ "action": "edit", "terrain": "water", "name": "sea" }));
    c.ok("terrain", json!({ "action": "move", "terrain": "sea", "to": 0 }));
    c.ok("terrain", json!({ "action": "remove", "terrain": "sea" }));
    let tile = c.ok("terrain", json!({ "action": "get_tile", "tile": 1 }));
    assert!(tile["result"]["mask"].as_i64().unwrap() >= 0, "{tile}");
    c.ok("terrain", json!({ "action": "set_tile", "tile": 1, "weight": 2.0, "symmetry": 1 }));
    c.ok("terrain", json!({ "action": "set_options", "edge_same": true, "seed": 42 }));
    let p = c.ok("terrain_paint", json!({ "action": "rect", "terrain": "grass", "x": 0, "y": 0, "w": 24, "h": 16 }));
    assert_eq!(p["missing_tile_cells"], 0, "{p}");
    c.ok("terrain_paint", json!({ "action": "paint", "terrain": 1, "points": [[28, 28]], "brush_size": 1 }));
    c.ok("terrain_paint", json!({ "action": "erase", "points": [[28, 28]] }));
    c.ok("terrain_paint", json!({ "action": "flood", "terrain": 1, "x": 30, "y": 30 }));
    let pick = c.ok("terrain_paint", json!({ "action": "pick", "x": 2, "y": 2 }));
    assert_eq!(pick["terrain"], 1);
    c.ok("terrain_paint", json!({ "action": "override", "x": 0, "y": 0, "tile": 1 }));
    c.ok("terrain", json!({ "action": "clear_overrides" }));
    c.ok("terrain", json!({ "action": "rebuild" }));
    let m = c.ok("terrain", json!({ "action": "missing" }));
    assert!(m["result"]["missing_cells"].is_u64());
    let r = c.call("render", json!({}));
    assert_eq!(Client::images(&r).len(), 1);
}

#[test]
fn slices_user_data_tween_lua() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 16, "height": 16 }));
    let s = c.ok("slice", json!({ "action": "add", "name": "button", "bounds": { "x": 1, "y": 1, "w": 10, "h": 8 } }));
    assert_eq!(s["slices"][0]["name"], "button");
    let s = c.ok(
        "slice",
        json!({ "action": "set", "slice": "button", "center": { "x": 2, "y": 2, "w": 6, "h": 4 }, "pivot": [5, 8] }),
    );
    assert_eq!(s["slices"][0]["center"], json!({ "x": 2, "y": 2, "w": 6, "h": 4 }));
    assert_eq!(s["slices"][0]["pivot"], json!([5, 8]));
    let s = c.ok(
        "slice",
        json!({ "action": "set", "slice": 0, "bounds": { "x": 0, "y": 0, "w": 12, "h": 12 }, "clear_pivot": true }),
    );
    assert_eq!(s["slices"][0]["bounds"]["w"], 12);
    assert!(s["slices"][0]["pivot"].is_null());
    c.ok(
        "user_data",
        json!({ "action": "set", "target": "slice", "slice": "button", "text": "ui", "color": "#ff0000" }),
    );
    let s = c.ok("slice", json!({ "action": "list" }));
    assert_eq!(s["slices"][0]["user_data"]["text"], "ui");
    c.ok("slice", json!({ "action": "remove", "slice": "button" }));

    c.ok("user_data", json!({ "target": "sprite", "text": "made by an agent" }));
    let u = c.ok("user_data", json!({ "target": "sprite" }));
    assert_eq!(u["user_data"]["text"], "made by an agent");
    c.ok("user_data", json!({ "target": "layer", "layer": 0, "text": "base", "color": "#00ff00" }));
    assert_eq!(c.ok("user_data", json!({ "target": "layer" }))["user_data"]["color"], "#00ff00ff");
    c.ok("user_data", json!({ "target": "cel", "text": "cel note" }));

    // Tween: move a square 8 px right over 5 frames, then bake.
    c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [0, 0], "to": [3, 3], "color": "#ffffff", "filled": true }),
    );
    c.ok("frame", json!({ "action": "add", "count": 4, "frame": 0 }));
    c.ok("tween", json!({ "action": "set_keys", "frame": 0, "values": { "x": 0, "y": 0 } }));
    let t = c.ok("tween", json!({ "action": "set_key", "frame": 4, "property": "x", "value": 8, "ease": "out_quad" }));
    assert!(t["tracks"]["x"].as_array().unwrap().len() == 2, "{t}");
    c.ok(
        "tween",
        json!({ "action": "set_ease", "frame": 0, "property": "x", "ease": "bezier", "bezier": [0.4, 0.0, 0.2, 1.0] }),
    );
    c.ok("tween", json!({ "action": "set_pivot", "x": 2, "y": 2 }));
    c.ok("tween", json!({ "action": "reset_pivot" }));
    c.ok("tween", json!({ "action": "set_options", "snap_scale": true }));
    c.ok("tween", json!({ "action": "copy_keys", "from": 0, "to": 0 }));
    let i = c.ok("tween", json!({ "action": "info", "frame": 4 }));
    assert_eq!(i["at_frame"]["x"], 8.0, "{i}");
    c.ok("tween", json!({ "action": "move_key", "property": "x", "from": 4, "to": 4 }));
    c.ok("tween", json!({ "action": "bake" }));
    assert_eq!(c.ok("get_pixel", json!({ "x": 8, "y": 0, "frame": 4 }))["color"], "#ffffffff");
    assert_eq!(c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 4 }))["color"], "#00000000");

    // Lua.
    let r = c.ok("run_lua", json!({ "source": "sprite:setPixel(15, 15, 0xFF00FFFF)\nprint(sprite.width, sprite.layerCount)\napp.activeFrame = 2" }));
    assert_eq!(r["output"], "16\t1");
    assert_eq!(r["changed_document"], true);
    assert_eq!(r["active_frame"], 1);
    assert_eq!(c.ok("get_pixel", json!({ "x": 15, "y": 15, "frame": 1 }))["color"], "#ff00ffff");
    let e = c.err("run_lua", json!({ "source": "sprite:setPixel(0, 0, 0xFF0000FF)\nerror('boom')" }));
    assert!(e.contains("boom"), "{e}");
    assert_eq!(
        c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 1 }))["color"],
        "#00000000",
        "a failed script changes nothing"
    );
    std::fs::write(c.path("s.lua"), "print('from file')").unwrap();
    assert_eq!(c.ok("run_lua", json!({ "path": "s.lua" }))["output"], "from file");
    let n = undo_steps(&mut c);
    c.ok("undo", json!({}));
    assert_eq!(undo_steps(&mut c), n - 1);
}

#[test]
fn files_export_and_godot() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 8, "name": "coin" }));
    c.ok("draw_shape", json!({ "shape": "ellipse", "from": [1, 1], "to": [6, 6], "color": "#ffcc00", "filled": true }));
    c.ok("frame", json!({ "action": "duplicate" }));
    c.ok("transform", json!({ "action": "flip_horizontal", "frame": 1 }));
    c.ok("tag", json!({ "action": "add", "name": "spin", "from": 0, "to": 1 }));
    let e = c.err("doc_save", json!({}));
    assert!(e.contains("pass path"), "{e}");
    let e = c.err("doc_save", json!({ "path": "coin.png" }));
    assert!(e.contains("export"), "{e}");
    let s = c.ok("doc_save", json!({ "path": "coin.aseprite" }));
    assert!(s["bytes"].as_u64().unwrap() > 100);
    assert_eq!(c.ok("doc_list", json!({}))["documents"][0]["dirty"], false);
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#f00" }));
    assert_eq!(c.ok("doc_list", json!({}))["documents"][0]["dirty"], true);
    let e = c.err("doc_close", json!({}));
    assert!(e.contains("unsaved"), "{e}");
    c.ok("doc_save", json!({}));
    c.ok("doc_save", json!({ "path": "copy.aseprite", "copy": true }));
    assert!(c.ok("doc_list", json!({}))["documents"][0]["path"].as_str().unwrap().ends_with("coin.aseprite"));
    c.ok("doc_close", json!({}));
    let o = c.ok("doc_open", json!({ "path": "coin.aseprite" }));
    assert_eq!(o["frames"], 2);
    assert_eq!(o["tags"], 1);
    // The duplicated frame was active when the pixel was drawn.
    assert_eq!(c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 1 }))["color"], "#ff0000ff");

    // Exports.
    let r = c.ok("export", json!({ "format": "png", "path": "out/coin.png", "scale": 4 }));
    assert_eq!(r["files_written"], 1);
    assert_eq!(png_size(&c.path("out/coin.png")), (32, 32));
    c.ok("export", json!({ "format": "gif", "path": "out/coin.gif", "scale": 2 }));
    assert_eq!(&std::fs::read(c.path("out/coin.gif")).unwrap()[..3], b"GIF");
    c.ok("export", json!({ "format": "apng", "path": "out/coin-anim.png" }));
    let r = c.ok(
        "export",
        json!({ "format": "sheet", "path": "out/sheet.png", "layout": "horizontal", "padding": 1, "json": true }),
    );
    assert!(c.path("out/sheet.json").is_file(), "{r}");
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(c.path("out/sheet.json")).unwrap()).unwrap();
    assert_eq!(meta["meta"]["frameTags"][0]["name"], "spin");
    c.ok("export", json!({ "format": "sheet", "path": "out/grid.png", "layout": "grid", "columns": 1, "trim": "per_frame", "extrude": 1 }));
    let r = c.ok("export", json!({ "format": "png_sequence", "path": "out/seq" }));
    assert!(c.path("out/seq-001.png").is_file() && c.path("out/seq-002.png").is_file(), "{r}");
    let r = c.ok("export", json!({ "format": "gif", "path": "out/tags.gif", "split_tags": true, "dry_run": true }));
    assert_eq!(r["outputs"], 1);
    c.ok("export", json!({ "format": "png", "path": "out/layer.png", "layers": "single", "layer": 0, "tag": "spin" }));

    // Re-import what we exported.
    let g = c.ok("doc_open", json!({ "path": "out/coin.gif" }));
    assert_eq!(g["frames"], 2);
    let s = c.ok("doc_open", json!({ "path": "out/sheet.json" }));
    assert_eq!(s["frames"], 2);
    assert_eq!(s["tags"], 1);
    let p = c.ok("doc_open", json!({ "path": "out/coin.png" }));
    assert_eq!(p["width"], 32);
    let gr = c.ok("doc_open", json!({ "path": "out/coin.png", "cell_width": 16 }));
    assert_eq!(gr["frames"], 4);
    c.ok("layer", json!({ "doc": "d2", "action": "import_png", "path": "out/seq-001.png", "name": "imported" }));

    // Godot.
    std::fs::create_dir_all(c.path("game")).unwrap();
    std::fs::write(c.path("game/project.godot"), "config_version=5\n").unwrap();
    let pv = c.call(
        "export_godot",
        json!({ "doc": "d2", "project_root": c.p("game"), "tres_path": "art/coin.tres", "preview": true }),
    );
    assert_eq!(pv["isError"], false, "{pv}");
    let r = c.ok("export_godot", json!({ "doc": "d2", "project_root": c.p("game"), "tres_path": "art/coin.tres", "tscn": true, "padding": 1, "save_settings": true }));
    let tres = std::fs::read_to_string(c.path("game/art/coin.tres")).unwrap();
    assert!(tres.contains("SpriteFrames") && tres.contains("\"spin\""), "{r}\n{tres}");
    assert!(r["files"].as_array().unwrap().iter().any(|f| f.as_str().unwrap().ends_with(".png")), "{r}");
    // Saved settings make a re-export argument-free.
    c.ok("export_godot", json!({ "doc": "d2" }));
    let e =
        c.err("export_godot", json!({ "doc": "d2", "project_root": c.p("game"), "tres_path": c.p("elsewhere.tres") }));
    assert!(!e.is_empty());

    for d in ["d2", "d3", "d4", "d5", "d6"] {
        c.ok("doc_close", json!({ "doc": d, "discard_changes": true }));
    }
    c.ok("doc_new", json!({ "width": 32, "height": 16 }));
    c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 8 }));
    c.ok("terrain", json!({ "action": "create", "mode": "blob", "name": "rock" }));
    c.ok("terrain", json!({ "action": "generate_template", "reduced": true }));
    c.ok("terrain", json!({ "action": "assign_masks" }));
    c.ok("terrain_paint", json!({ "action": "rect", "terrain": 1, "x": 0, "y": 0, "w": 32, "h": 16 }));
    let pv = c.ok(
        "export_godot_tileset",
        json!({ "project_root": c.p("game"), "tres_path": "art/rock.tres", "preview": true }),
    );
    assert!(pv["description"].is_string());
    let r = c.ok(
        "export_godot_tileset",
        json!({ "project_root": c.p("game"), "tres_path": "art/rock.tres", "collision": "full", "write_scene": true }),
    );
    let tres = std::fs::read_to_string(c.path("game/art/rock.tres")).unwrap();
    assert!(tres.contains("TileSet") && tres.contains("terrain_set_0"), "{r}\n{tres}");
}
