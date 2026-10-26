//! Previews: a filter preview with `region` is cropped to it, and the Godot tileset preview describes the tile shape.

mod common;
use common::Client;
use serde_json::{Value, json};

fn payload(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn filter_preview_with_a_region_is_cropped_before_scaling() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 60, "height": 40 }));
    c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [10, 5], "to": [29, 14], "color": "#ff0000", "filled": true }),
    );
    let region = json!({ "x": 8, "y": 4, "w": 30, "h": 12 });
    let r = c.call("filter", json!({ "kind": "invert", "region": region, "preview": true, "scale": 8 }));
    assert_eq!(r["isError"], json!(false), "{r}");
    let imgs = Client::images(&r);
    assert_eq!(imgs.len(), 1);
    assert_eq!((imgs[0].0, imgs[0].1), (30 * 8, 12 * 8), "the preview is the region times the scale");
    let v = payload(&r);
    assert_eq!(v["region"], region, "{v}");
    assert_eq!(v["image_size"], json!([240, 96]), "{v}");
    assert!(v["note"].as_str().unwrap().contains("cropped to the region"), "{v}");
    // The crop shows the region: its pixel (2, 1) is canvas (10, 5), the rectangle's corner, inverted to cyan.
    let (w, _, px) = &imgs[0];
    let at = |x: usize, y: usize| &px[(y * *w as usize + x) * 4..(y * *w as usize + x) * 4 + 4];
    assert_eq!(at(2 * 8 + 3, 8 + 3), [0x00, 0xff, 0xff, 0xff]);
    // Nothing changed.
    assert_eq!(c.pixel(10, 5), "#ff0000ff");
    // Without a region the whole canvas is shown and no region is reported.
    let r = c.call("filter", json!({ "kind": "invert", "preview": true, "scale": 2 }));
    let imgs = Client::images(&r);
    assert_eq!((imgs[0].0, imgs[0].1), (120, 80));
    assert!(payload(&r).get("region").is_none());
    // A region partly outside the canvas is clipped, and the preview matches the clipped size.
    let r = c.call(
        "filter",
        json!({ "kind": "invert", "region": { "x": 50, "y": 30, "w": 30, "h": 30 }, "preview": true, "scale": 3 }),
    );
    let imgs = Client::images(&r);
    assert_eq!((imgs[0].0, imgs[0].1), (30, 30));
}

#[test]
fn godot_tileset_preview_describes_the_isometric_shape() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 400, "height": 300 }));
    c.ok(
        "tilemap",
        json!({ "action": "add_layer", "name": "floor", "cell_w": 96, "cell_h": 48, "tile_height": 96, "offset": [592, 360] }),
    );
    c.ok("tileset", json!({ "action": "add_tile", "rows": vec!["r".repeat(96); 96], "legend": { "r": "#ff0000" } }));
    c.ok("tilemap", json!({ "action": "stamp", "i": 0, "j": 0, "tile": 1 }));
    let root = c.path("game");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("project.godot"), "config_version=5\n").unwrap();
    let pv =
        c.ok("export_godot_tileset", json!({ "project_root": c.p("game"), "tres_path": "iso.tres", "preview": true }));
    let t = &pv["tileset"];
    assert_eq!(t["tile_shape"], "isometric", "{pv}");
    assert_eq!(t["tile_layout"], "diamond_down", "{pv}");
    assert_eq!(t["tile_size"], json!([96, 48]), "{pv}");
    assert_eq!(t["cell_size"], json!([96, 48]), "{pv}");
    assert_eq!(t["tile_image_size"], json!([96, 96]), "{pv}");
    assert_eq!(t["texture_origins"], json!([{ "texture_origin": [0, 24], "tiles": 1, "first_tiles": [1] }]), "{pv}");
    assert_eq!(
        t["scene_layers"],
        json!([{ "layer": 1, "name": "floor", "position": [592, 360], "y_sort_enabled": true, "position_written": true }]),
        "{pv}"
    );
    // It matches what the real export writes.
    c.ok("export_godot_tileset", json!({ "project_root": c.p("game"), "tres_path": "iso.tres", "write_scene": true }));
    let tres = std::fs::read_to_string(root.join("iso.tres")).unwrap();
    assert!(tres.contains("tile_shape = 1") && tres.contains("tile_size = Vector2i(96, 48)"), "{tres}");
    assert!(tres.contains("/texture_origin = Vector2i(0, 24)"), "{tres}");
    let tscn = std::fs::read_to_string(root.join("iso.tscn")).unwrap();
    assert!(tscn.contains("position = Vector2(592, 360)") && tscn.contains("y_sort_enabled = true"), "{tscn}");
}

#[test]
fn godot_tileset_preview_of_a_square_tileset_says_square() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 64, "height": 64 }));
    c.ok("tilemap", json!({ "action": "add_layer", "tile_width": 16 }));
    c.ok("tileset", json!({ "action": "add_tile" }));
    let root = c.path("game");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("project.godot"), "config_version=5\n").unwrap();
    let pv =
        c.ok("export_godot_tileset", json!({ "project_root": c.p("game"), "tres_path": "sq.tres", "preview": true }));
    assert_eq!(pv["tileset"]["tile_shape"], "square", "{pv}");
    assert_eq!(pv["tileset"]["tile_size"], json!([16, 16]), "{pv}");
    assert!(pv["tileset"]["tile_layout"].is_null(), "{pv}");
}
