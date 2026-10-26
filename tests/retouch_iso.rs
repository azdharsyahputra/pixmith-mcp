//! Blur / Jumble brushes (`retouch`) and the isometric tile selection (`select` action `iso_tile`).

mod common;
use common::Client;
use serde_json::json;

fn undo_steps(c: &mut Client) -> u64 {
    c.ok("history", json!({}))["undo_steps"].as_u64().unwrap()
}

/// A 8x8 red/blue checkerboard on the first layer.
fn checker(c: &mut Client) {
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    let (mut red, mut blue) = (vec![], vec![]);
    for y in 0..8 {
        for x in 0..8 {
            if (x + y) % 2 == 0 { red.push([x, y]) } else { blue.push([x, y]) }
        }
    }
    c.ok("draw_pixels", json!({ "points": red, "color": "#ff0000" }));
    c.ok("draw_pixels", json!({ "points": blue, "color": "#0000ff" }));
}

fn all_pixels(c: &mut Client) -> Vec<String> {
    let r = c.ok("get_pixels", json!({ "format": "hex" }));
    r["rows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|row| row.as_array().unwrap().iter())
        .map(|v| v.as_str().unwrap().to_string())
        .collect()
}

#[test]
fn blur_averages_the_neighbourhood_in_one_undo_step() {
    let mut c = Client::start(&[]);
    checker(&mut c);
    let n = undo_steps(&mut c);
    let r = c.ok("retouch", json!({ "kind": "blur", "points": [[4, 4]], "size": 1, "strength": 100 }));
    assert_eq!(r["pixels_changed"], 1, "{r}");
    assert_eq!(c.pixel(4, 4), "#8e0071ff");
    // Neighbours stay.
    assert_eq!(c.pixel(5, 4), "#0000ffff");
    assert_eq!(undo_steps(&mut c), n + 1);
    c.ok("undo", json!({}));
    assert_eq!(c.pixel(4, 4), "#ff0000ff");
}

#[test]
fn blur_is_clipped_to_the_selection() {
    let mut c = Client::start(&[]);
    checker(&mut c);
    c.ok("select", json!({ "action": "rect", "x": 0, "y": 0, "w": 4, "h": 8 }));
    c.ok("retouch", json!({ "kind": "blur", "points": [[0, 4], [7, 4]], "size": 3, "strength": 100 }));
    assert_eq!(c.pixel(6, 4), "#ff0000ff", "outside the selection is untouched");
    assert_ne!(c.pixel(2, 4), "#ff0000ff");
}

#[test]
fn jumble_is_repeatable_and_only_moves_pixels() {
    let mut c = Client::start(&[]);
    checker(&mut c);
    let before = all_pixels(&mut c);
    let args = json!({ "kind": "jumble", "points": [[1, 1], [6, 6]], "size": 5, "strength": 100, "seed": 7 });
    let r = c.ok("retouch", args.clone());
    assert!(r["pixels_changed"].as_u64().unwrap() > 0, "{r}");
    let once = all_pixels(&mut c);
    assert_ne!(once, before);
    let sorted = |v: &Vec<String>| {
        let mut v = v.clone();
        v.sort();
        v
    };
    assert_eq!(sorted(&once), sorted(&before), "jumble swaps pixels, it never makes new colors");
    c.ok("undo", json!({}));
    assert_eq!(all_pixels(&mut c), before);
    c.ok("retouch", args);
    assert_eq!(all_pixels(&mut c), once, "same seed, same result");
}

#[test]
fn retouch_rejects_bad_input() {
    let mut c = Client::start(&[]);
    checker(&mut c);
    assert!(c.err("retouch", json!({ "kind": "smudge", "points": [[1, 1]] })).contains("invalid arguments"));
    assert!(c.err("retouch", json!({ "kind": "blur", "points": [] })).contains("1-65536"));
}

/// Pixels of the document that a full-canvas fill reaches (only the selection takes paint), per row.
fn painted_rows(c: &mut Client) -> Vec<(usize, usize, usize)> {
    c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [0, 0], "to": [63, 31], "color": "#ff0000", "filled": true }),
    );
    let px = all_pixels(c);
    let mut rows = vec![];
    for y in 0..32 {
        let row = &px[y * 64..(y + 1) * 64];
        let xs: Vec<usize> = (0..64).filter(|&x| row[x] == "#ff0000ff").collect();
        if let (Some(&l), Some(&r)) = (xs.first(), xs.last()) {
            assert_eq!(xs.len(), r - l + 1, "row {y} is one run");
            rows.push((y, l, r));
        }
    }
    rows
}

#[test]
fn iso_tile_selects_the_diamond_under_a_pixel() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 64, "height": 32 }));
    let n = undo_steps(&mut c);
    // Default 32x16 grid: cell (1, 0) is the diamond whose box is x 16..48, y 8..24 (30 pixels wide at the middle).
    let s = c.ok("select", json!({ "action": "iso_tile", "x": 32, "y": 16 }));
    assert_eq!(s["bounds"], json!({ "x": 17, "y": 8, "w": 30, "h": 16 }), "{s}");
    assert_eq!(
        s["iso_cell"],
        json!({ "col": 1, "row": 0, "tile_w": 32, "tile_h": 16, "offset_x": 0, "offset_y": 0, "grid_source": "default", "layer": null })
    );
    assert_eq!(undo_steps(&mut c), n + 1);
    let rows = painted_rows(&mut c);
    let widths: Vec<usize> = rows.iter().map(|r| r.2 - r.1 + 1).collect();
    assert_eq!(widths, [2, 6, 10, 14, 18, 22, 26, 30, 30, 26, 22, 18, 14, 10, 6, 2]);
    assert_eq!(rows[0], (8, 31, 32));
    assert_eq!(widths.iter().sum::<usize>(), 256);
}

#[test]
fn iso_tile_by_col_row_offsets_and_modes() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 64, "height": 32 }));
    // The same cell by column and row.
    let a = c.ok("select", json!({ "action": "iso_tile", "col": 1, "row": 0 }));
    assert_eq!(a["bounds"], json!({ "x": 17, "y": 8, "w": 30, "h": 16 }), "{a}");
    // Add the neighbouring cell (0, 0) and subtract it again.
    let b = c.ok("select", json!({ "action": "iso_tile", "col": 0, "row": 0, "mode": "add" }));
    assert_eq!(b["bounds"], json!({ "x": 1, "y": 0, "w": 46, "h": 24 }), "{b}");
    let d = c.ok("select", json!({ "action": "iso_tile", "x": 16, "y": 8, "mode": "subtract" }));
    assert_eq!(d["bounds"], json!({ "x": 17, "y": 8, "w": 30, "h": 16 }), "{d}");
    // Offset and a different tile size move the grid: cell (0, 0) has its box at the offset.
    let e = c.ok(
        "select",
        json!({ "action": "iso_tile", "col": 0, "row": 0, "tile_w": 16, "tile_h": 8, "offset_x": 3, "offset_y": 2 }),
    );
    assert_eq!(e["bounds"], json!({ "x": 4, "y": 2, "w": 14, "h": 8 }), "{e}");
    // x/y and col/row do not mix; one of them is required.
    assert!(c.err("select", json!({ "action": "iso_tile" })).contains("`x` and `y`"));
    assert!(
        c.err("select", json!({ "action": "iso_tile", "x": 1, "y": 1, "col": 0, "row": 0 }))
            .contains("`col` and `row`")
    );
}
