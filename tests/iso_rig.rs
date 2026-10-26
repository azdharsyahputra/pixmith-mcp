//! Isometric tilemaps (the user's 96 x 48 grid at (592, 360)) and the tween rig (parented layers).

mod common;
use common::Client;
use serde_json::{Value, json};

fn undo_steps(c: &mut Client) -> u64 {
    c.ok("history", json!({}))["undo_steps"].as_u64().unwrap()
}

fn alpha(c: &mut Client, x: i32, y: i32) -> String {
    c.pixel(x, y)[7..].to_string()
}

/// Rows of `w` x `h` characters filled with `ch`.
fn solid(w: usize, h: usize, ch: char) -> Vec<String> {
    vec![ch.to_string().repeat(w); h]
}

// MARK: Isometric tilemaps

/// The user's map: 1000 x 600 canvas, an isometric layer of 96 x 48 diamonds whose cell (0, 0) has its box at
/// (592, 360), with 96 x 96 tile images (taller than the diamond). Tile 1 is solid red.
fn user_map(c: &mut Client) -> Value {
    c.ok("doc_new", json!({ "width": 1000, "height": 600 }));
    let l = c.ok(
        "tilemap",
        json!({ "action": "add_layer", "name": "floor", "cell_w": 96, "cell_h": 48, "tile_height": 96, "offset": [592, 360] }),
    );
    assert_eq!(l["layer"], 1, "{l}");
    assert_eq!(l["tile_size"], json!([96, 96]), "{l}");
    assert_eq!(l["iso"], json!({ "cell_w": 96, "cell_h": 48, "layout": "diamond_down", "offset": [592, 360] }), "{l}");
    let t = c.ok("tileset", json!({ "action": "add_tile", "rows": solid(96, 96, 'r'), "legend": { "r": "#ff0000" } }));
    assert_eq!(t["tile"], 1, "{t}");
    l
}

#[test]
fn the_users_iso_map_cell_math_pivot_and_pixels() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    // doc_info shows the isometric layer and tileset.
    let info = c.ok("doc_info", json!({ "sections": ["layers", "tilesets"] }));
    assert_eq!(info["layers"][1]["kind"], "tilemap", "{info}");
    assert_eq!(info["layers"][1]["iso"]["offset"], json!([592, 360]));
    assert_eq!(info["tilesets"][0]["iso"], json!({ "cell_w": 96, "cell_h": 48, "layout": "diamond_down" }));
    let n = undo_steps(&mut c);
    let s = c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 1 }));
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(
        s["cell"],
        json!({ "tile": 1, "flip_x": false, "flip_y": false, "diagonal": false, "i": 0, "j": 0 }),
        "{s}"
    );
    // Cell (0, 0): box at (592, 360), diamond center (640, 384). The 96 x 96 image's pivot defaults to
    // (48, 72), so its top-left corner lands at (592, 312).
    assert_eq!(c.pixel(592, 312), "#ff0000ff");
    assert_eq!(c.pixel(687, 407), "#ff0000ff");
    assert_eq!(alpha(&mut c, 591, 312), "00");
    assert_eq!(alpha(&mut c, 688, 407), "00");
    assert_eq!(alpha(&mut c, 592, 311), "00");
    // Cell (1, 0) is half a cell right and down; (0, 1) half a cell left and down.
    c.ok("tilemap", json!({ "action": "stamp", "i": 1, "j": 0, "tile": 1 }));
    c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 1, "tile": 1 }));
    assert_eq!(c.pixel(687 + 48, 407 + 24), "#ff0000ff", "bottom-right corner of (1, 0)'s image");
    assert_eq!(alpha(&mut c, 687 + 49, 407 + 24), "00");
    assert_eq!(c.pixel(592 - 48, 312 + 24), "#ff0000ff", "top-left corner of (0, 1)'s image");
    assert_eq!(alpha(&mut c, 592 - 49, 312 + 24), "00");
    // get_tile by cell, by the pixel of a cell's center, and what is seen there.
    let g = c.ok("tilemap", json!({ "action": "get_tile", "i": 1, "j": 0 }));
    assert_eq!(g["cell"]["tile"], 1, "{g}");
    let g = c.ok("tilemap", json!({ "action": "get_tile", "x": 640, "y": 384 }));
    assert_eq!((g["cell"]["i"].as_i64(), g["cell"]["j"].as_i64()), (Some(0), Some(0)), "{g}");
    assert_eq!(g["seen"]["tile"], 1, "{g}");
    let g = c.ok("tilemap", json!({ "action": "get_tile", "x": 688, "y": 408 }));
    assert_eq!((g["cell"]["i"].as_i64(), g["cell"]["j"].as_i64()), (Some(1), Some(0)), "{g}");
    let g = c.ok("tilemap", json!({ "action": "get_tile", "i": 5, "j": 5 }));
    assert_eq!(g["cell"]["tile"], 0);
    assert_eq!(g["iso"]["offset"], json!([592, 360]));
    // The grid lists cells: rows = j, columns = i.
    let g = c.ok("tilemap", json!({ "action": "grid" }));
    assert_eq!(g["first_cell"], json!([0, 0]), "{g}");
    assert_eq!(g["rows"][0].as_str().unwrap().trim(), "1 1", "{g}");
    assert!(g["rows"][1].as_str().unwrap().starts_with("1 "), "{g}");
    // Clearing a cell and a cell outside the canvas (cells may lie anywhere).
    c.ok("tilemap", json!({ "action": "stamp", "i": 1, "j": 0, "tile": 0 }));
    assert_eq!(alpha(&mut c, 687 + 48, 407 + 24), "00");
    c.ok("tilemap", json!({ "action": "stamp", "i": -3, "j": 2, "tile": 1 }));
    assert_eq!(c.ok("tilemap", json!({ "action": "get_tile", "i": -3, "j": 2 }))["cell"]["tile"], 1);
    // Flips yes, the diagonal flip no.
    c.ok("tilemap", json!({ "action": "stamp", "i": 2, "j": 0, "tile": 1, "flip_x": true }));
    let e = c.err("tilemap", json!({ "action": "stamp", "i": 2, "j": 0, "tile": 1, "diagonal": true }));
    assert!(e.contains("diagonal"), "{e}");
    let e = c.err("tilemap", json!({ "action": "stamp", "tile": 1 }));
    assert!(e.contains("`i` and `j`"), "{e}");
    let e = c.err("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 9 }));
    assert!(e.contains("does not exist"), "{e}");
}

#[test]
fn select_iso_tile_and_iso_grid_share_the_layers_grid() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    // No iso_grid setting was made: select iso_tile uses the active isometric layer's grid.
    let s = c.ok("select", json!({ "action": "iso_tile", "x": 640, "y": 384 }));
    let cell = &s["iso_cell"];
    assert_eq!((cell["col"].as_i64(), cell["row"].as_i64()), (Some(0), Some(0)), "{s}");
    assert_eq!((cell["tile_w"].as_i64(), cell["tile_h"].as_i64()), (Some(96), Some(48)), "{s}");
    assert_eq!((cell["offset_x"].as_i64(), cell["offset_y"].as_i64()), (Some(592), Some(360)), "{s}");
    assert_eq!(cell["grid_source"], "layer");
    let b = &s["bounds"];
    assert_eq!((b["y"].as_i64(), b["h"].as_i64()), (Some(360), Some(48)), "{s}");
    assert!(b["x"].as_i64().unwrap() >= 592 && b["x"].as_i64().unwrap() + b["w"].as_i64().unwrap() <= 688, "{s}");
    let s = c.ok("select", json!({ "action": "iso_tile", "col": 2, "row": 1 }));
    // Cell (2, 1): box at (592 + 48, 360 + 3 * 24).
    assert_eq!(s["bounds"]["y"], 432, "{s}");
    assert!(s["bounds"]["x"].as_i64().unwrap() >= 640, "{s}");
    // iso_grid reads the layer too and its set moves the layer.
    let g = c.ok("iso_grid", json!({}));
    assert_eq!(
        (g["tile_w"].as_i64(), g["tile_h"].as_i64(), g["offset_x"].as_i64(), g["offset_y"].as_i64()),
        (Some(96), Some(48), Some(592), Some(360)),
        "{g}"
    );
    assert_eq!((g["source"].as_str(), g["layer"].as_u64()), (Some("layer"), Some(1)), "{g}");
    let n = undo_steps(&mut c);
    let g = c.ok("iso_grid", json!({ "action": "set", "offset_y": 336 }));
    assert_eq!(g["offset_y"], 336, "{g}");
    assert_eq!(undo_steps(&mut c), n + 1, "moving the grid is one undo step");
    assert_eq!(c.ok("tilemap", json!({ "action": "offset" }))["iso"]["offset"], json!([592, 336]));
    let s = c.ok("select", json!({ "action": "iso_tile", "col": 0, "row": 0 }));
    assert_eq!(s["iso_cell"]["offset_y"], 336, "{s}");
    // The cell size belongs to the tileset.
    let e = c.err("iso_grid", json!({ "action": "set", "tile_w": 64 }));
    assert!(e.contains("set_iso"), "{e}");
    // Explicit arguments still win.
    let s = c.ok(
        "select",
        json!({ "action": "iso_tile", "col": 0, "row": 0, "tile_w": 32, "tile_h": 16, "offset_x": 0, "offset_y": 0 }),
    );
    assert_eq!(s["iso_cell"]["tile_w"], 32, "{s}");
    // A layer that is not isometric is a mistake when asked for by name.
    let e = c.err("select", json!({ "action": "iso_tile", "col": 0, "row": 0, "layer": 0 }));
    assert!(e.contains("not an isometric"), "{e}");
}

#[test]
fn without_an_iso_layer_the_servers_iso_grid_setting_is_used() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 64, "height": 32 }));
    let g = c.ok("iso_grid", json!({}));
    assert_eq!(g["source"], "default", "{g}");
    c.ok("iso_grid", json!({ "action": "set", "tile_w": 96, "tile_h": 48, "offset_x": 592, "offset_y": 360 }));
    let g = c.ok("iso_grid", json!({}));
    assert_eq!((g["source"].as_str(), g["offset_x"].as_i64()), (Some("setting"), Some(592)), "{g}");
    let s = c.ok("select", json!({ "action": "iso_tile", "col": 0, "row": 0 }));
    assert_eq!(s["iso_cell"]["grid_source"], "setting");
    assert_eq!(s["iso_cell"]["offset_y"], 360);
}

#[test]
fn tilemap_offset_places_layers_at_other_elevations() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 1 }));
    assert_eq!(c.pixel(592, 312), "#ff0000ff");
    let n = undo_steps(&mut c);
    // A ledge half a cell higher: the same grid 24 px up.
    let r = c.ok("tilemap", json!({ "action": "offset", "offset": [592, 336] }));
    assert_eq!(r["iso"]["offset"], json!([592, 336]), "{r}");
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(alpha(&mut c, 592, 335 + 24), "ff");
    assert_eq!(c.pixel(592, 288), "#ff0000ff");
    assert_eq!(alpha(&mut c, 592, 287), "00");
    c.ok("undo", json!({}));
    assert_eq!(c.pixel(592, 312), "#ff0000ff");
    // Reading is free; non-iso layers have no offset.
    let r = c.ok("tilemap", json!({ "action": "offset" }));
    assert_eq!(r["iso"]["offset"], json!([592, 360]));
    let e = c.err("tilemap", json!({ "action": "offset", "layer": 0 }));
    assert!(e.contains("is not an isometric tilemap layer") || e.contains("not a tilemap layer"), "{e}");
}

#[test]
fn tileset_pivots_texture_origin_and_shape() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 1 }));
    let p = c.ok("tileset", json!({ "action": "get_pivot", "tile": 1 }));
    assert_eq!(p["pivot"], json!([48, 72]), "{p}");
    assert_eq!(p["texture_origin"], json!([0, 24]), "{p}");
    assert_eq!(p["custom"], false);
    let n = undo_steps(&mut c);
    let p = c.ok("tileset", json!({ "action": "set_pivot", "tile": 1, "pivot": [40, 60] }));
    assert_eq!((p["pivot"].clone(), p["custom"].clone()), (json!([40, 60]), json!(true)), "{p}");
    assert_eq!(p["texture_origin"], json!([-8, 12]));
    assert_eq!(undo_steps(&mut c), n + 1);
    // The image now sits with pivot (40, 60) on the diamond center (640, 384): top-left (600, 324).
    assert_eq!(c.pixel(600, 324), "#ff0000ff");
    assert_eq!(alpha(&mut c, 599, 324), "00");
    let all = c.ok("tileset", json!({ "action": "pivots" }));
    assert_eq!(all["pivots"].as_array().unwrap().len(), 1, "{all}");
    assert_eq!(all["pivots"][0]["custom"], true);
    let o = c.ok("tileset", json!({ "action": "texture_origin", "tile": 1 }));
    assert_eq!(o["texture_origin"], json!([-8, 12]));
    let p = c.ok("tileset", json!({ "action": "clear_pivot", "tile": 1 }));
    assert_eq!(p["pivot"], json!([48, 72]), "{p}");
    assert_eq!(c.pixel(592, 312), "#ff0000ff");
    // Errors: tile 0 and a missing tile.
    assert!(c.err("tileset", json!({ "action": "set_pivot", "tile": 0, "pivot": [1, 1] })).contains("tile"));
    assert!(c.err("tileset", json!({ "action": "get_pivot" })).contains("`tile`"));
    // The shape: a tileset that no layer shows becomes isometric (and square again), a used one is refused.
    c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 16, "layer": 0 }));
    let ts = c.ok("tileset", json!({ "action": "list" }));
    assert_eq!(ts["tilesets"][1]["iso"], Value::Null, "{ts}");
    assert!(
        c.err("tileset", json!({ "action": "get_pivot", "tileset": 1, "tile": 1 })).contains("not isometric")
            || c.err("tileset", json!({ "action": "get_pivot", "tileset": 1, "tile": 0 })).contains("not isometric")
    );
    let r = c.ok("tileset", json!({ "action": "set_iso", "tileset": 1, "cell_w": 32, "cell_h": 16 }));
    assert_eq!(r["tileset"]["iso"], json!({ "cell_w": 32, "cell_h": 16, "layout": "diamond_down" }), "{r}");
    let r = c.ok("tileset", json!({ "action": "set_iso", "tileset": 1, "cell_w": 0 }));
    assert_eq!(r["tileset"]["iso"], Value::Null, "{r}");
    let e = c.err("tileset", json!({ "action": "set_iso", "tileset": 0, "cell_w": 0 }));
    assert!(e.contains("no layer shows"), "{e}");
}

#[test]
fn terrain_paint_works_on_an_isometric_layer() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 200, "height": 100 }));
    c.ok("tilemap", json!({ "action": "add_layer", "cell_w": 32, "cell_h": 16, "offset": [100, 4] }));
    c.ok("terrain", json!({ "action": "create", "mode": "sides", "name": "grass", "color": "#33aa33" }));
    let t = c.ok("terrain", json!({ "action": "generate_template" }));
    assert!(t["result"]["tiles_added"].as_u64().unwrap() >= 16, "{t}");
    c.ok("terrain", json!({ "action": "assign_masks" }));
    // The centers of cells (0, 0) and (1, 0): boxes at (100, 4) and (116, 12).
    let p = c.ok("terrain_paint", json!({ "action": "paint", "terrain": "grass", "points": [[116, 12], [132, 20]] }));
    assert_eq!(p["missing_tile_cells"], 0, "{p}");
    let a = c.ok("tilemap", json!({ "action": "get_tile", "i": 0, "j": 0 }));
    let b = c.ok("tilemap", json!({ "action": "get_tile", "i": 1, "j": 0 }));
    assert!(a["cell"]["tile"].as_u64().unwrap() > 0 && b["cell"]["tile"].as_u64().unwrap() > 0, "{a} {b}");
    assert_ne!(a["cell"]["tile"], b["cell"]["tile"], "the two ends of a pair get different edge tiles");
    assert_eq!(c.ok("terrain_paint", json!({ "action": "pick", "x": 116, "y": 12 }))["terrain"], 1);
    // Painting by pixel follows the layer's offset: the same pixel after moving the grid hits another cell.
    let p = c.ok("terrain_paint", json!({ "action": "erase", "points": [[116, 12]] }));
    assert_eq!(p["missing_tile_cells"], 0, "{p}");
    assert_eq!(c.ok("tilemap", json!({ "action": "get_tile", "i": 0, "j": 0 }))["cell"]["tile"], 0);
    // The dual grid needs a half-cell shift that diamonds do not have.
    let e = c.err("terrain", json!({ "action": "set_mode", "mode": "corners" }));
    assert!(!e.is_empty());
}

#[test]
fn the_iso_map_survives_a_save_and_reopen() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    c.ok("tilemap", json!({ "action": "stamp", "i": 1, "j": 1, "tile": 1 }));
    c.ok("tileset", json!({ "action": "set_pivot", "tile": 1, "pivot": [40, 60] }));
    let path = c.p("map.pxmt");
    c.ok("doc_save", json!({ "path": &path }));
    c.ok("doc_close", json!({}));
    c.ok("doc_open", json!({ "path": &path }));
    let info = c.ok("doc_info", json!({ "sections": ["layers", "tilesets"] }));
    assert_eq!(info["layers"][1]["iso"]["offset"], json!([592, 360]), "{info}");
    assert_eq!(info["tilesets"][0]["iso"]["cell_h"], 48);
    assert_eq!(c.ok("tilemap", json!({ "action": "get_tile", "layer": 1, "i": 1, "j": 1 }))["cell"]["tile"], 1);
    assert_eq!(c.ok("tileset", json!({ "action": "get_pivot", "tile": 1 }))["pivot"], json!([40, 60]));
}

#[test]
fn export_godot_tileset_writes_the_isometric_tileset_and_the_map_with_its_offset() {
    let mut c = Client::start(&[]);
    user_map(&mut c);
    c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 1 }));
    c.ok("tilemap", json!({ "action": "stamp", "i": 1, "j": 0, "tile": 1 }));
    let root = c.path("game");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("project.godot"), "config_version=5\n").unwrap();
    let pv =
        c.ok("export_godot_tileset", json!({ "project_root": c.p("game"), "tres_path": "iso.tres", "preview": true }));
    assert!(pv["description"].is_string(), "{pv}");
    c.ok("export_godot_tileset", json!({ "project_root": c.p("game"), "tres_path": "iso.tres", "write_scene": true }));
    let tres = std::fs::read_to_string(root.join("iso.tres")).unwrap();
    assert!(tres.contains("tile_shape = 1"), "{tres}");
    assert!(tres.contains("tile_size = Vector2i(96, 48)"), "{tres}");
    assert!(tres.contains("texture_region_size = Vector2i(96, 96)"), "{tres}");
    assert!(tres.contains("/texture_origin = Vector2i(0, 24)"), "{tres}");
    let tscn = std::fs::read_to_string(root.join("iso.tscn")).unwrap();
    assert!(tscn.contains("position = Vector2(592, 360)"), "{tscn}");
    assert!(tscn.contains("y_sort_enabled = true"), "{tscn}");
}

#[test]
fn iso_layer_argument_errors() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 64, "height": 64 }));
    let e = c.err("tilemap", json!({ "action": "add_layer", "cell_h": 8, "tile_width": 16 }));
    assert!(e.contains("cell_w"), "{e}");
    let e = c.err("tilemap", json!({ "action": "add_layer", "offset": [1, 1], "tile_width": 16 }));
    assert!(e.contains("cell_w"), "{e}");
    let e = c.err("tilemap", json!({ "action": "add_layer", "cell_w": 1 }));
    assert!(e.contains("at least 2"), "{e}");
    // A square layer still takes pixels (and cells i, j that are pixel / tile size).
    c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 8 }));
    c.ok("tileset", json!({ "action": "add_tile" }));
    c.ok("tilemap", json!({ "action": "stamp", "x": 9, "y": 1, "tile": 1 }));
    assert_eq!(c.ok("tilemap", json!({ "action": "get_tile", "i": 1, "j": 0 }))["cell"]["tile"], 1);
    let e = c.err("tilemap", json!({ "action": "get_tile", "i": 1 }));
    assert!(e.contains("`x` and `y`"), "{e}");
}

// MARK: Tween rig

/// A 32 x 32 document with "thigh" (layer 0) and "shin" (layer 1) blocks, three frames, and thigh keys rotating
/// 0 -> 90 -> 0 degrees about its hip (15, 4). The shin's pivot is the knee (15, 12).
fn leg(c: &mut Client) {
    c.ok("doc_new", json!({ "width": 32, "height": 32 }));
    c.ok("layer", json!({ "action": "set", "layer": 0, "name": "thigh" }));
    c.ok("draw_shape", json!({ "shape": "rectangle", "from": [14, 4], "to": [16, 11], "color": "#ff0000", "filled": true, "layer": 0 }));
    c.ok("layer", json!({ "action": "add", "name": "shin" }));
    c.ok("draw_shape", json!({ "shape": "rectangle", "from": [14, 12], "to": [16, 19], "color": "#0000ff", "filled": true, "layer": "shin" }));
    c.ok("frame", json!({ "action": "add", "count": 2 }));
    c.ok("tween", json!({ "action": "set_pivot", "layer": "thigh", "x": 15, "y": 4 }));
    c.ok("tween", json!({ "action": "set_pivot", "layer": "shin", "x": 15, "y": 12 }));
    for (f, v) in [(0, 0.0), (1, 90.0), (2, 0.0)] {
        c.ok("tween", json!({ "action": "set_key", "layer": "thigh", "frame": f, "property": "rotation", "value": v }));
    }
}

fn near(a: &Value, want: f64) -> bool {
    (a.as_f64().unwrap() - want).abs() < 0.01
}

#[test]
fn a_child_follows_its_parent_in_the_tween_rig() {
    let mut c = Client::start(&[]);
    leg(&mut c);
    // Unparented, the shin stays where it is in frame 1.
    assert_eq!(alpha(&mut c, 4, 5), "00");
    let n = undo_steps(&mut c);
    let r = c.ok("tween", json!({ "action": "set_parent", "layer": "shin", "parent": "thigh" }));
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(r["rig"]["parent"], 0, "{r}");
    assert_eq!(r["rig"]["parent_name"], "thigh");
    let p = c.ok("tween", json!({ "action": "get_parent", "layer": "thigh" }));
    assert_eq!(p["rig"]["parent"], Value::Null);
    assert_eq!(p["rig"]["children"], json!([1]), "{p}");
    let k = c.ok("tween", json!({ "action": "children", "layer": 0 }));
    assert_eq!(k["rig"]["children"], json!([1]));
    // The shin has no keys of its own but follows the thigh: at frame 1 its knee swung to (7, 4).
    let t = c.ok("tween", json!({ "action": "composed_transform", "layer": "shin", "frame": 1 }));
    let m = &t["result"];
    assert!(near(&m["rotation"], 90.0), "{t}");
    assert!(near(&m["pivot"][0], 7.0) && near(&m["pivot"][1], 4.0), "{t}");
    assert!(near(&m["scale_x"], 1.0));
    // The matrix is the thigh's (the shin's own transform is the identity).
    let th = c.ok("tween", json!({ "action": "composed_transform", "layer": "thigh", "frame": 1 }));
    for i in 0..6 {
        assert!(near(&m["matrix"][i], th["result"]["matrix"][i].as_f64().unwrap()), "{t} {th}");
    }
    // The picture agrees: the blue shin now lies to the left of the hip.
    assert_eq!(c.ok("get_pixel", json!({ "x": 4, "y": 5, "frame": 1 }))["color"], "#0000ffff");
    assert_eq!(c.ok("get_pixel", json!({ "x": 4, "y": 5, "frame": 0 }))["color"], "#00000000");
    // doc_info shows the link.
    let info = c.ok("doc_info", json!({ "sections": ["layers"] }));
    assert_eq!(info["layers"][1]["tween_parent"], 0, "{info}");
    // The effective range of the child is the parent's.
    assert_eq!(r["rig"]["effective_range"], json!([0, 2]), "{r}");
}

#[test]
fn rig_links_refuse_cycles_and_unlink_and_survive_a_save() {
    let mut c = Client::start(&[]);
    leg(&mut c);
    c.ok("tween", json!({ "action": "set_parent", "layer": "shin", "parent": "thigh" }));
    let e = c.err("tween", json!({ "action": "set_parent", "layer": "thigh", "parent": "shin" }));
    assert!(e.contains("cannot have"), "{e}");
    let e = c.err("tween", json!({ "action": "set_parent", "layer": "thigh", "parent": "thigh" }));
    assert!(e.contains("cannot have"), "{e}");
    assert!(c.err("tween", json!({ "action": "set_parent", "layer": "thigh" })).contains("`parent`"));
    // The link is saved with the document.
    let path = c.p("leg.pxmt");
    c.ok("doc_save", json!({ "path": &path }));
    c.ok("doc_close", json!({}));
    c.ok("doc_open", json!({ "path": &path }));
    let p = c.ok("tween", json!({ "action": "get_parent", "layer": "shin" }));
    assert_eq!(p["rig"]["parent"], 0, "{p}");
    // Unlinking is one undo step and leaves the shin without any transform.
    let n = undo_steps(&mut c);
    let r = c.ok("tween", json!({ "action": "clear_parent", "layer": "shin" }));
    assert_eq!(r["rig"]["parent"], Value::Null);
    assert_eq!(undo_steps(&mut c), n + 1);
    let e = c.err("tween", json!({ "action": "composed_transform", "layer": "shin", "frame": 1 }));
    assert!(e.contains("no tween keys"), "{e}");
}

#[test]
fn baking_all_is_rig_aware() {
    let mut c = Client::start(&[]);
    leg(&mut c);
    c.ok("tween", json!({ "action": "set_parent", "layer": "shin", "parent": "thigh" }));
    let before: Vec<Value> = (0..3)
        .map(|f| {
            c.ok(
                    "get_pixels",
                    json!({ "format": "hex", "region": { "x": 0, "y": 0, "w": 32, "h": 32 }, "frame": f }),
                )["rows"]
                    .clone()
        })
        .collect();
    let r = c.ok("tween", json!({ "action": "bake_all" }));
    assert!(r["result"]["layers_baked"].as_u64().unwrap() >= 1, "{r}");
    for (f, want) in before.iter().enumerate() {
        let got =
            c.ok("get_pixels", json!({ "format": "hex", "region": { "x": 0, "y": 0, "w": 32, "h": 32 }, "frame": f }))
                ["rows"]
                .clone();
        assert_eq!(&got, want, "frame {f} looks the same after baking");
    }
    // The baked layers are plain cels now.
    assert_eq!(c.ok("get_pixel", json!({ "x": 4, "y": 5, "frame": 1 }))["color"], "#0000ffff");
}
