//! cel block reports, `layer import_sheet`, default save path of imported documents, `iso_grid`, `--allow-dir`
//! and what `run_lua` can reach.

mod common;
use common::{Client, write_png};
use serde_json::json;

#[test]
fn cel_copy_and_paste_report_the_block() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    c.ok("layer", json!({ "action": "add" }));
    c.ok("frame", json!({ "action": "add", "count": 2 }));
    for l in 0..2 {
        for f in 0..3 {
            c.ok("draw_pixels", json!({ "layer": l, "frame": f, "points": [[l, f]], "color": "#f00" }));
        }
    }
    let r = c.ok("cel", json!({ "action": "copy", "layer": 0, "to_layer": 1, "frame": 0, "to_frame": 2 }));
    assert_eq!(r["copied"], json!({ "layers": [0, 1], "frames": [0, 2], "cels": 6, "with_image": 6 }), "{r}");
    assert!(r["summary"].as_str().unwrap().contains("copied 2 layer(s) x 3 frame(s) = 6 cel(s)"), "{r}");
    c.ok("frame", json!({ "action": "add", "count": 3, "frame": 2 }));
    let p = c.ok("cel", json!({ "action": "paste", "layer": 0, "frame": 3 }));
    assert_eq!(p["pasted"], json!({ "layers": [0, 1], "frames": [3, 5], "cels": 6, "with_image": 6 }), "{p}");
    let m = c.ok("cel", json!({ "action": "move", "layer": 0, "frame": 3, "to_frame": 5, "dl": 1, "copy": true }));
    assert_eq!(m["duplicated"], json!({ "layers": [1, 1], "frames": [3, 5], "cels": 3, "with_image": 3 }), "{m}");
}

fn sheet(c: &Client, name: &str) -> std::path::PathBuf {
    // 8x4: two 4x4 cells, red then blue.
    let mut px = Vec::new();
    for _ in 0..4 {
        for x in 0..8 {
            px.extend_from_slice(if x < 4 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
        }
    }
    let p = c.path(name);
    write_png(&p, 8, 4, &px);
    p
}

#[test]
fn import_sheet_fills_consecutive_frames_in_one_undo_step() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let png = sheet(&c, "s.png");
    let steps = c.ok("history", json!({}))["undo_steps"].as_u64().unwrap();
    let r = c.ok(
        "layer",
        json!({ "action": "import_sheet", "path": png.to_string_lossy(), "cell_width": 4, "cell_height": 4, "layer": 0 }),
    );
    assert_eq!(r["sheet"]["cells_imported"], 2, "{r}");
    assert_eq!(r["sheet"]["frames_created"], 1, "{r}");
    assert_eq!(c.ok("history", json!({}))["undo_steps"].as_u64().unwrap(), steps + 1);
    assert_eq!(c.ok("get_pixel", json!({ "x": 1, "y": 1, "frame": 0 }))["color"], "#ff0000ff");
    assert_eq!(c.ok("get_pixel", json!({ "x": 1, "y": 1, "frame": 1 }))["color"], "#0000ffff");
    c.ok("undo", json!({}));
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["frames"], 1);
    // A new layer, starting at frame 1, with a square cell.
    let r = c.ok(
        "layer",
        json!({ "action": "import_sheet", "path": png.to_string_lossy(), "cell_width": 4, "new_layer": true, "name": "walk", "frame": 1 }),
    );
    assert_eq!(r["sheet"]["first_frame"], 1, "{r}");
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["frames"], 3);
    assert_eq!(c.ok("get_pixel", json!({ "x": 0, "y": 0, "frame": 2 }))["color"], "#0000ffff");
    let e = c.err("layer", json!({ "action": "import_sheet", "path": png.to_string_lossy() }));
    assert!(e.contains("cell_width"), "{e}");
}

#[test]
fn an_imported_document_saves_next_to_its_source() {
    let mut c = Client::start(&[]);
    let png = sheet(&c, "walk.png");
    let id = c.ok("doc_open", json!({ "path": png.to_string_lossy() }))["doc"].as_str().unwrap().to_string();
    let s = c.ok("doc_save", json!({ "doc": id }));
    assert!(s["saved"].as_str().unwrap().ends_with("walk.pxmt"), "{s}");
    assert!(c.path("walk.pxmt").exists());
    assert!(s["note"].as_str().unwrap().contains("next to the imported file"), "{s}");
    // A new document still needs a path.
    c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    assert!(c.err("doc_save", json!({ "doc": "d2" })).contains("pass path"));
}

#[test]
fn headless_save_replace_closes_a_clean_holder_only() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    let file = c.p("a.pxmt");
    c.ok("doc_save", json!({ "doc": "d1", "path": file }));
    c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[0, 0]], "color": "#f00" }));
    let e = c.err("doc_save", json!({ "doc": "d2", "path": file, "replace": true }));
    assert!(e.contains("unsaved changes") && e.contains("d1"), "{e}");
    c.ok("doc_save", json!({ "doc": "d1" }));
    let r = c.ok("doc_save", json!({ "doc": "d2", "path": file, "replace": true }));
    assert_eq!(r["closed_documents"], json!(["d1"]), "{r}");
}

#[test]
fn iso_grid_is_the_default_of_iso_tile() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 400, "height": 200 }));
    let g = c.ok("iso_grid", json!({}));
    assert_eq!(
        (g["tile_w"].as_i64(), g["tile_h"].as_i64(), g["is_default"].as_bool()),
        (Some(32), Some(16), Some(true))
    );
    let g = c.ok("iso_grid", json!({ "action": "set", "tile_w": 96, "tile_h": 48, "offset_x": 100, "offset_y": 20 }));
    assert_eq!(g["tile_w"], 96, "{g}");
    assert_eq!(g["is_default"], false);
    // Cell (0, 0) now has its box at (100, 20), 96 x 48; its widest row is 94 px.
    let s = c.ok("select", json!({ "action": "iso_tile", "col": 0, "row": 0 }));
    assert_eq!(s["iso_cell"]["tile_w"], 96, "{s}");
    assert_eq!(s["bounds"], json!({ "x": 101, "y": 20, "w": 94, "h": 48 }), "{s}");
    // A single call can still override one value.
    let s = c.ok("select", json!({ "action": "iso_tile", "col": 0, "row": 0, "offset_x": 0 }));
    assert_eq!(s["bounds"]["x"], 1, "{s}");
    // A partial set keeps the rest.
    let g = c.ok("iso_grid", json!({ "action": "set", "offset_y": 40 }));
    assert_eq!((g["tile_w"].as_i64(), g["offset_x"].as_i64(), g["offset_y"].as_i64()), (Some(96), Some(100), Some(40)));
    assert!(c.err("iso_grid", json!({ "tile_w": 8 })).contains("need action set"));
    let g = c.ok("iso_grid", json!({ "action": "reset" }));
    assert_eq!(g["is_default"], true);
}

#[test]
fn allow_dir_takes_reads_and_exports_but_not_saves() {
    let scratch = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let dir_arg = scratch.path().to_string_lossy().into_owned();
    let root_arg = root.path().to_string_lossy().into_owned();
    let mut c = Client::start(&["--root", &root_arg, "--allow-dir", &dir_arg]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let out = scratch.path().join("out.png").to_string_lossy().into_owned();
    c.ok("export", json!({ "format": "png", "path": out }));
    assert!(scratch.path().join("out.png").exists());
    // Reading from there works too.
    assert!(c.ok("doc_open", json!({ "path": out }))["doc"].is_string());
    // Saving a document there does not.
    let e = c.err("doc_save", json!({ "doc": "d1", "path": scratch.path().join("x.pxmt").to_string_lossy() }));
    assert!(e.contains("only take reads and exports"), "{e}");
    // Neither does a folder that was not listed.
    let other = tempfile::tempdir().unwrap();
    let e = c.err("export", json!({ "format": "png", "path": other.path().join("o.png").to_string_lossy() }));
    assert!(e.contains("outside the allowed root"), "{e}");
}

#[test]
fn run_lua_reads_images_inside_allow_paths_and_sees_only_its_own_documents() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    let png = sheet(&c, "in.png");
    let dir = c.path("");
    let src = format!(
        "local img = Image{{ fromFile = {:?} }}\nprint(img.width, img.height)\nprint(#app.sprites)",
        png.to_string_lossy()
    );
    let r = c.ok("run_lua", json!({ "source": src, "allow_paths": [dir.to_string_lossy()] }));
    assert_eq!(r["output"], "8\t4\n1", "{r}");
    // Without allow_paths the read is denied.
    assert!(c.err("run_lua", json!({ "source": src })).contains("script failed"));
    // A second open document is not part of app.sprites.
    c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    let r = c.ok("run_lua", json!({ "doc": "d1", "source": "print(#app.sprites)" }));
    assert_eq!(r["output"], "1", "{r}");
}
