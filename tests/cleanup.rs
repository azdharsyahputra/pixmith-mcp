//! Clean Edges, Remove Background, Sprite Lint, Trace Pixel Art, Frame Difference and Warp on small synthetic
//! sprites. Every document is made in a temporary directory that is also the server's HOME.

mod common;
use common::{Client, pseudo_pixel, write_png};
use serde_json::{Value, json};

const T: [u8; 4] = [0, 0, 0, 0];
const K: [u8; 4] = [20, 20, 30, 255]; // outline
const R: [u8; 4] = [200, 60, 50, 255]; // body
const S: [u8; 4] = [235, 190, 160, 255]; // skin
const W: [u8; 4] = [250, 250, 250, 255]; // highlight
const P: [u8; 4] = [225, 225, 220, 255]; // paper halo
const KEY: &[(char, [u8; 4])] = &[('K', K), ('R', R), ('S', S), ('W', W), ('P', P)];

/// Writes `rows` (one character per pixel, `.` transparent) as a PNG and opens it. Returns the document id.
fn open_art(c: &mut Client, name: &str, rows: &[&str], key: &[(char, [u8; 4])]) -> String {
    let (w, h) = (rows[0].len(), rows.len());
    let mut px = Vec::new();
    for r in rows {
        assert_eq!(r.len(), w);
        for ch in r.chars() {
            px.extend_from_slice(&if ch == '.' { T } else { key.iter().find(|k| k.0 == ch).expect("key").1 });
        }
    }
    let path = c.path(name);
    write_png(&path, w as u32, h as u32, &px);
    c.ok("doc_open", json!({ "path": path.to_string_lossy() }))["doc"].as_str().unwrap().to_string()
}

fn undo_steps(c: &mut Client) -> u64 {
    c.ok("history", json!({}))["undo_steps"].as_u64().unwrap()
}

fn alpha(c: &mut Client, x: i32, y: i32) -> String {
    c.pixel(x, y)[7..].to_string()
}

fn creature() -> Vec<&'static str> {
    vec![
        "............",
        "...KKKKKK...",
        "..KRRRRRRK..",
        ".KRRRRWRRRK.",
        ".KRRKKRRRRK.",
        ".KRRRRRRRRKK",
        ".KRRRRRRRRKS",
        ".KRRRRRRRRK.",
        "..KRRRRRRK..",
        "...KKKKKK...",
        "K...........",
        "............",
    ]
}

const HALO_ART: [&str; 10] = [
    "..........",
    ".PPPPPPPP.",
    ".PWWWWWWP.",
    ".PWKKKKWP.",
    ".PWKRRKWP.",
    ".PWKRRKWP.",
    ".PWKKKKWP.",
    ".PWWWWWWP.",
    ".PPPPPPPP.",
    "..........",
];

// MARK: Clean Edges

#[test]
fn clean_edges_default_passes_remove_specks_and_keep_small_interior_details() {
    let mut c = Client::start(&[]);
    open_art(&mut c, "creature.png", &creature(), KEY);
    assert_eq!(alpha(&mut c, 0, 10), "ff", "the speck is there");
    let n = undo_steps(&mut c);
    let r = c.ok("filter", json!({ "kind": "clean_edges" }));
    assert_eq!(r["filter"], "clean_edges", "{r}");
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(undo_steps(&mut c), n + 1, "one undo step");
    assert_eq!(alpha(&mut c, 0, 10), "00", "the detached speck is gone");
    // The eye (2 px, touches the body), the 1 px highlight and the 1 px finger on the outline all survive.
    assert_eq!(c.pixel(4, 4), "#14141eff");
    assert_eq!(c.pixel(5, 4), "#14141eff");
    assert_eq!(c.pixel(6, 3), "#fafafaff");
    assert_eq!(c.pixel(11, 6), "#ebbea0ff");
    assert_eq!(c.pixel(10, 5), "#14141eff");
    // Nothing left to clean: no new undo step.
    let again = c.ok("filter", json!({ "kind": "clean_edges" }));
    assert_eq!(again["changed"], false, "{again}");
    assert_eq!(undo_steps(&mut c), n + 1);
    c.ok("undo", json!({}));
    assert_eq!(alpha(&mut c, 0, 10), "ff");
}

#[test]
fn clean_edges_halo_pass_peels_the_paper_rings() {
    let mut c = Client::start(&[]);
    open_art(&mut c, "halo.png", &HALO_ART, KEY);
    let r = c.ok("filter", json!({ "kind": "clean_edges", "halo": true }));
    assert_eq!(r["changed"], true, "{r}");
    for y in 1..=8 {
        for x in 1..=8 {
            let band = x <= 2 || x >= 7 || y <= 2 || y >= 7;
            assert_eq!(alpha(&mut c, x, y) == "00", band, "({x},{y})");
        }
    }
    assert_eq!(c.pixel(3, 3), "#14141eff");
    assert_eq!(c.pixel(4, 4), "#c83c32ff");
    // One ring alone is not "much lighter than what is behind it" here (white behind the paper): nothing goes.
    c.ok("undo", json!({}));
    let one = c.ok("filter", json!({ "kind": "clean_edges", "halo_rings": 1 }));
    assert_eq!(one["changed"], false, "{one}");
    assert_eq!(alpha(&mut c, 1, 1), "ff");
    // A single ring of paper in front of the dark outline goes with one ring.
    let mut c = Client::start(&[]);
    open_art(&mut c, "ring.png", &[".....", ".PPP.", ".PKP.", ".PPP.", "....."], KEY);
    let r = c.ok("filter", json!({ "kind": "clean_edges", "halo_rings": 1, "halo_gain": 40 }));
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(alpha(&mut c, 1, 1), "00");
    assert_eq!(c.pixel(2, 2), "#14141eff");
}

#[test]
fn clean_edges_thin_outline_and_holes() {
    let mut c = Client::start(&[]);
    open_art(
        &mut c,
        "thick.png",
        &[
            "..........",
            ".KKKKKKKK.",
            ".KKKKKKKK.",
            ".KKRRRRKK.",
            ".KKRRRRKK.",
            ".KKRRRRKK.",
            ".KKRRRRKK.",
            ".KKKKKKKK.",
            ".KKKKKKKK.",
            "..........",
        ],
        KEY,
    );
    c.ok("filter", json!({ "kind": "clean_edges", "thin_outline": true }));
    assert_eq!(alpha(&mut c, 1, 1), "00", "the outer outline ring is gone");
    assert_eq!(c.pixel(2, 2), "#14141eff", "the inner ring is the 1 px outline");
    assert_eq!(c.pixel(3, 3), "#c83c32ff");
    // A thicker outline than `thickness` is left alone.
    let mut c = Client::start(&[]);
    open_art(
        &mut c,
        "thick2.png",
        &["........", ".KKKKKK.", ".KKKKKK.", ".KKRRKK.", ".KKRRKK.", ".KKKKKK.", ".KKKKKK.", "........"],
        KEY,
    );
    let r = c.ok("filter", json!({ "kind": "clean_edges", "thin_outline": true, "thickness": 1 }));
    assert_eq!(r["changed"], false, "{r}");
    // Holes: only on request, and only enclosed ones below the size.
    let mut c = Client::start(&[]);
    open_art(&mut c, "hole.png", &["......", ".RRRR.", ".R.RR.", ".RRRR.", "......"], KEY);
    let none = c.ok("filter", json!({ "kind": "clean_edges" }));
    assert_eq!(none["changed"], false, "holes are kept by default: {none}");
    c.ok("filter", json!({ "kind": "clean_edges", "holes": 2 }));
    assert_eq!(alpha(&mut c, 2, 2), "ff");
}

#[test]
fn clean_edges_previews_and_targets_every_frame() {
    let mut c = Client::start(&[]);
    open_art(&mut c, "creature.png", &creature(), KEY);
    c.ok("frame", json!({ "action": "duplicate" }));
    let n = undo_steps(&mut c);
    let r = c.call("filter", json!({ "kind": "clean_edges", "preview": true }));
    assert_eq!(r["isError"], false, "{r}");
    assert_eq!(Client::images(&r).len(), 1);
    assert_eq!(undo_steps(&mut c), n, "a preview changes nothing");
    assert_eq!(alpha(&mut c, 0, 10), "ff");
    // target layer: every frame of the layer in one step.
    c.ok("filter", json!({ "kind": "clean_edges", "target": "layer" }));
    for f in [0, 1] {
        let p = c.ok("get_pixel", json!({ "x": 0, "y": 10, "frame": f }));
        assert_eq!(p["color"], "#00000000", "frame {f}");
    }
    assert_eq!(undo_steps(&mut c), n + 1);
}

// MARK: Lint

#[test]
fn lint_reports_every_category_with_capped_positions_and_changes_nothing() {
    let mut c = Client::start(&[]);
    let rows = [
        "...........",
        ".PPPPPPPPP.",
        ".PKKKKKKKP.",
        ".PKRRRRRKP.",
        ".PKR.RWRKP.",
        ".PKRRRRRKP.",
        ".PKKKKKKKP.",
        ".PPPPPPPPP.",
        "...........",
        "..........K",
    ];
    open_art(&mut c, "lint.png", &rows, KEY);
    // One semi-transparent pixel and one color that is not in the palette.
    c.ok("draw_pixels", json!({ "points": [[4, 3]], "color": "#c83c3280" }));
    c.ok("draw_pixels", json!({ "points": [[3, 5]], "color": "#010203" }));
    let n = undo_steps(&mut c);
    let r = c.ok("lint", json!({ "palette": true }));
    assert_eq!(undo_steps(&mut c), n, "lint is read-only");
    assert_eq!(r["cels_checked"], 1, "{r}");
    let cel = &r["cels"][0];
    assert_eq!((cel["layer"].as_u64(), cel["frame"].as_u64()), (Some(0), Some(0)), "{r}");
    assert_eq!(cel["holes"]["count"], 1, "{r}");
    assert_eq!(cel["holes"]["items"][0], json!({ "x": 4, "y": 4, "size": 1 }));
    assert_eq!(cel["specks"]["items"][0], json!({ "x": 10, "y": 9, "size": 1 }));
    assert_eq!(cel["semi_transparent"]["items"], json!([{ "x": 4, "y": 3, "size": 1 }]), "{r}");
    assert_eq!(cel["off_palette"]["items"], json!([{ "x": 3, "y": 5, "size": 1 }]), "{r}");
    assert!(cel["strays"]["items"].as_array().unwrap().iter().any(|s| s["x"] == 3 && s["y"] == 5), "{r}");
    assert!(cel["light_edge"]["count"].as_u64().unwrap() >= 20, "the paper band: {r}");
    assert!(r["total"].as_u64().unwrap() >= 25, "{r}");
    assert_eq!(r["totals_by_category"]["holes"], 1);
    // Without palette=true there is no off_palette report; `list` caps the positions but not the counts.
    let r = c.ok("lint", json!({ "list": 3 }));
    let cel = &r["cels"][0];
    assert_eq!(cel["off_palette"]["count"], 0);
    assert_eq!(cel["light_edge"]["items"].as_array().unwrap().len(), 3, "{r}");
    assert!(cel["light_edge"]["count"].as_u64().unwrap() > 3);
}

#[test]
fn lint_targets_frames_and_finds_nothing_in_a_clean_creature() {
    let mut c = Client::start(&[]);
    let mut rows = creature();
    rows[10] = "............";
    open_art(&mut c, "clean.png", &rows, KEY);
    let r = c.ok("lint", json!({}));
    for cat in ["holes", "specks", "semi_transparent", "light_edge"] {
        assert_eq!(r["cels"][0][cat]["count"], 0, "{cat}: {r}");
    }
    c.ok("frame", json!({ "action": "duplicate" }));
    c.ok("draw_pixels", json!({ "points": [[0, 11]], "color": "#ff00ff", "frame": 1 }));
    let r = c.ok("lint", json!({ "target": "layer" }));
    assert_eq!(r["cels_checked"], 2, "{r}");
    assert_eq!(r["cels"][0]["specks"]["count"], 0);
    assert_eq!(r["cels"][1]["specks"]["items"][0], json!({ "x": 0, "y": 11, "size": 1 }), "{r}");
    assert_eq!(r["cels"][1]["frame"], 1);
    let r = c.ok("lint", json!({ "target": "frame", "frame": 1 }));
    assert_eq!(r["cels_checked"], 1, "{r}");
}

// MARK: Remove Background

const BG_KEY: &[(char, [u8; 4])] = &[('W', [255, 255, 255, 255]), ('K', K), ('R', R), ('D', [204, 204, 204, 255])];

const BG_ART: [&str; 10] = [
    "WWWWWWWWWWWW",
    "WWWWWWWWWWWW",
    "WWKKKKKKKWWW",
    "WWKRRRRRKWWW",
    "WWKRWWRRKWWW",
    "WWKRRRRRKWWW",
    "WWKKKKKKKWWW",
    "WWDDDDDDDWWW",
    "WWWWWWWWWWWW",
    "WWWWWWWWWWWW",
];

#[test]
fn remove_background_floods_from_the_border_and_keeps_enclosed_colors() {
    let mut c = Client::start(&[]);
    open_art(&mut c, "bg.png", &BG_ART, BG_KEY);
    let n = undo_steps(&mut c);
    let r = c.ok("filter", json!({ "kind": "remove_background" }));
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["background"]["background"], 78, "{r}");
    assert_eq!(r["background"]["removed"], 78);
    assert_eq!(r["background"]["color"], "#ffffff");
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(alpha(&mut c, 0, 0), "00");
    assert_eq!(c.pixel(4, 4), "#ffffffff", "the enclosed eye stays");
    assert_eq!(alpha(&mut c, 3, 7), "ff", "the shadow stays without shadow=true");
    // global also removes the enclosed white.
    c.ok("undo", json!({}));
    let g = c.ok("filter", json!({ "kind": "remove_background", "global": true }));
    assert_eq!(g["background"]["global"], 2, "{g}");
    assert_eq!(alpha(&mut c, 4, 4), "00");
    // shadow removes the grey strip under the figure too.
    c.ok("undo", json!({}));
    let s = c.ok("filter", json!({ "kind": "remove_background", "shadow": true }));
    assert_eq!(s["background"]["shadow"], 7, "{s}");
    assert_eq!(alpha(&mut c, 3, 7), "00");
    assert_eq!(c.pixel(3, 3), "#c83c32ff");
}

#[test]
fn remove_background_previews_and_takes_a_color() {
    let mut c = Client::start(&[]);
    open_art(&mut c, "bg.png", &BG_ART, BG_KEY);
    let n = undo_steps(&mut c);
    let r = c.call("filter", json!({ "kind": "remove_background", "preview": true }));
    assert_eq!(Client::images(&r).len(), 1, "{r}");
    assert_eq!(undo_steps(&mut c), n);
    assert_eq!(alpha(&mut c, 0, 0), "ff");
    // An explicit background color that is not on the border removes nothing.
    let r = c.ok("filter", json!({ "kind": "remove_background", "color": "#00ff00" }));
    assert_eq!(r["changed"], false, "{r}");
    assert_eq!(undo_steps(&mut c), n);
    // tolerance 0 still matches exact white.
    let r = c.ok("filter", json!({ "kind": "remove_background", "tolerance": 0 }));
    assert_eq!(r["changed"], true, "{r}");
}

// MARK: Trace

/// Share of the original sprite's pixels found in the traced 16x16 image (hex rows of get_pixels).
fn match_share(c: &mut Client, sprite: &[[u8; 4]]) -> f64 {
    let r = c.ok("get_pixels", json!({ "format": "hex" }));
    let rows = r["rows"].as_array().unwrap();
    let mut ok = 0;
    for (i, want) in sprite.iter().enumerate() {
        let got = rows[i / 16][i % 16].as_str().unwrap();
        let g: Vec<f64> = (0..4).map(|k| u8::from_str_radix(&got[1 + 2 * k..3 + 2 * k], 16).unwrap() as f64).collect();
        let good = if want[3] == 0 {
            g[3] == 0.0
        } else {
            g[3] != 0.0 && (0..3).map(|k| (want[k] as f64 - g[k]).powi(2)).sum::<f64>().sqrt() <= 30.0
        };
        ok += good as usize;
    }
    ok as f64 / sprite.len() as f64
}

#[test]
fn doc_open_trace_grid_finds_the_grid_and_returns_the_report() {
    let mut c = Client::start(&[]);
    let (w, h, rgba, sprite) = pseudo_pixel();
    let path = c.path("ai.png");
    write_png(&path, w as u32, h as u32, &rgba);
    let r = c.ok(
        "doc_open",
        json!({ "path": path.to_string_lossy(), "format": "trace_grid", "remove_background": true, "shadow": true, "close_outline": true }),
    );
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(16), Some(16)), "{r}");
    let t = &r["trace"];
    let pitch = t["pitch"][0].as_f64().unwrap();
    assert!((pitch - 8.3).abs() < 0.4, "pitch {pitch}: {r}");
    assert_eq!(t["width"], 16);
    assert!(t["quality"].as_f64().unwrap() > 0.5, "{r}");
    assert!(t["palette"].as_array().unwrap().len() >= 3, "{r}");
    assert!(t["background"]["removed"].as_u64().unwrap() > 0, "{r}");
    assert_eq!(t["background"]["color"], "#ffffff");
    let share = match_share(&mut c, &sprite);
    assert!(share > 0.9, "only {share} of the pixels match");
    // The new document is an import: it is saved next to the source and is not dirty.
    assert_eq!(r["dirty"], false);
}

#[test]
fn doc_open_trace_grid_takes_a_fixed_pitch_and_a_palette() {
    let mut c = Client::start(&[]);
    let (w, h, rgba, _) = pseudo_pixel();
    let path = c.path("ai.png");
    write_png(&path, w as u32, h as u32, &rgba);
    let r = c.ok(
        "doc_open",
        json!({ "path": path.to_string_lossy(), "format": "trace_grid", "pitch": 8.3, "colors": 4, "tolerance": 30, "crop": false }),
    );
    assert_eq!(r["trace"]["pitch"], json!([8.3, 8.3]), "{r}");
    assert!(r["trace"]["palette"].as_array().unwrap().len() <= 4, "{r}");
    assert!(r["width"].as_u64().unwrap() >= 16, "{r}");
    let e = c.err("doc_open", json!({ "path": path.to_string_lossy(), "format": "trace_grid", "pitch": "wide" }));
    assert!(e.contains("pitch"), "{e}");
    let e = c.err("doc_open", json!({ "path": path.to_string_lossy(), "format": "trace_grid", "colors": "lots" }));
    assert!(e.contains("colors"), "{e}");
}

#[test]
fn trace_tool_resamples_an_open_cel_in_one_undo_step() {
    let mut c = Client::start(&[]);
    let (w, h, rgba, sprite) = pseudo_pixel();
    let path = c.path("ai.png");
    write_png(&path, w as u32, h as u32, &rgba);
    let r = c.ok("doc_open", json!({ "path": path.to_string_lossy() }));
    assert_eq!(r["width"].as_u64().unwrap() as usize, w);
    let n = undo_steps(&mut c);
    // detect_only and preview change nothing.
    let d = c.ok("trace", json!({ "detect_only": true }));
    assert!((d["pitch"][0].as_f64().unwrap() - 8.3).abs() < 0.4, "{d}");
    assert_eq!(d["applied"], false);
    let p = c.call("trace", json!({ "preview": true, "remove_background": true, "shadow": true }));
    assert_eq!(p["isError"], false, "{p}");
    assert_eq!(Client::images(&p).len(), 1);
    assert_eq!(undo_steps(&mut c), n);
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["width"].as_u64().unwrap() as usize, w);
    // Apply.
    let t = c.ok("trace", json!({ "remove_background": true, "shadow": true, "close_outline": true }));
    assert_eq!(t["applied"], true, "{t}");
    assert_eq!(t["canvas"], json!([16, 16]), "{t}");
    assert!((t["pitch"][0].as_f64().unwrap() - 8.3).abs() < 0.4, "{t}");
    assert_eq!(undo_steps(&mut c), n + 1);
    assert!(match_share(&mut c, &sprite) > 0.9);
    c.ok("undo", json!({}));
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["width"].as_u64().unwrap() as usize, w);
}

#[test]
fn trace_refuses_a_flat_image_and_leaves_the_document_alone() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 20, "height": 20, "background": "#336699" }));
    let n = undo_steps(&mut c);
    let e = c.err("trace", json!({}));
    assert!(!e.is_empty());
    assert_eq!(undo_steps(&mut c), n);
    let e = c.err("trace", json!({ "colors": "palette", "pitch": [3.0] }));
    assert!(e.contains("invalid arguments"), "{e}");
}

// MARK: Frame difference

/// A 3-frame, 8x8, two-layer document: "Layer 1" holds the walk parts, the second layer a constant dot.
/// Frame 1 differs from frame 0 in layer 0 by one changed, one disappeared and one appeared pixel.
fn walk(c: &mut Client) {
    c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    c.ok("draw_pixels", json!({ "points": [[1, 1], [2, 1]], "color": "#ff0000" }));
    c.ok("layer", json!({ "action": "add", "name": "dot" }));
    c.ok("draw_pixels", json!({ "points": [[7, 7]], "color": "#00ff00", "layer": "dot" }));
    c.ok("frame", json!({ "action": "duplicate", "frame": 0 }));
    c.ok("draw_pixels", json!({ "points": [[2, 1]], "color": "#0000ff", "layer": 0, "frame": 1 }));
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "transparent", "layer": 0, "frame": 1 }));
    c.ok("draw_pixels", json!({ "points": [[5, 5]], "color": "#ffff00", "layer": 0, "frame": 1 }));
}

#[test]
fn frame_diff_counts_changed_appeared_and_disappeared_pixels() {
    let mut c = Client::start(&[]);
    walk(&mut c);
    let n = undo_steps(&mut c);
    let d = c.ok("frame_diff", json!({ "frame": 1 }));
    assert_eq!(
        (d["changed"].as_u64(), d["appeared"].as_u64(), d["disappeared"].as_u64(), d["total"].as_u64()),
        (Some(1), Some(1), Some(1), Some(3)),
        "{d}"
    );
    assert_eq!(d["bounds"], json!({ "x": 1, "y": 1, "w": 5, "h": 5 }), "{d}");
    assert_eq!(d["reference_frame"], 0);
    assert_eq!(undo_steps(&mut c), n, "read-only");
    // Frame 0 wraps to the last frame: the roles of appeared and disappeared swap.
    let w = c.ok("frame_diff", json!({ "frame": 0 }));
    assert_eq!((w["appeared"].as_u64(), w["disappeared"].as_u64(), w["changed"].as_u64()), (Some(1), Some(1), Some(1)));
    assert_eq!(w["reference_frame"], 1);
    let nw = c.ok("frame_diff", json!({ "frame": 0, "wrap": false }));
    assert_eq!(nw["total"], 0, "{nw}");
    assert_eq!(nw["reference_frame"], Value::Null);
    assert_eq!(nw["bounds"], Value::Null);
    // A layer that did not move at all.
    let l = c.ok("frame_diff", json!({ "frame": 1, "layer": "dot" }));
    assert_eq!(l["total"], 0, "{l}");
    let l = c.ok("frame_diff", json!({ "frame": 1, "layer": 0 }));
    assert_eq!(l["total"], 3, "{l}");
    // A table over all frames.
    let all = c.ok("frame_diff", json!({ "all": true }));
    let rows = all["frames"].as_array().unwrap();
    assert_eq!(rows.len(), 2, "{all}");
    assert_eq!(rows[1]["total"], 3);
}

#[test]
fn frame_diff_image_and_render_diff_prev_tint_the_changed_pixels() {
    let mut c = Client::start(&[]);
    walk(&mut c);
    let r = c.call("frame_diff", json!({ "frame": 1, "image": true, "scale": 1, "background": "transparent" }));
    assert_eq!(r["isError"], false, "{r}");
    let imgs = Client::images(&r);
    assert_eq!(imgs.len(), 1);
    // The pixel that disappeared (1, 1) is transparent in the frame and tinted red.
    let at = |img: &(u32, u32, Vec<u8>), x: usize, y: usize| img.2[(y * img.0 as usize + x) * 4..][..4].to_vec();
    assert_eq!(at(&imgs[0], 1, 1)[..3], [255, 0, 0]);
    assert!(at(&imgs[0], 1, 1)[3] > 100);
    // Plain render: nothing there.
    let plain = c.call("render", json!({ "frame": 1, "scale": 1, "background": "transparent" }));
    let p = Client::images(&plain);
    assert_eq!(at(&p[0], 1, 1)[3], 0);
    assert!(plain["content"][0]["text"].as_str().unwrap().contains("\"frames\""));
    // render diff_prev=true draws the same overlay and reports the numbers.
    let d = c.call("render", json!({ "frame": 1, "scale": 1, "background": "transparent", "diff_prev": true }));
    let di = Client::images(&d);
    assert_eq!(at(&di[0], 1, 1), at(&imgs[0], 1, 1));
    let text: Value = serde_json::from_str(d["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text["frame_difference"][0]["total"], 3, "{text}");
    // The unchanged green dot is not tinted.
    assert_eq!(at(&di[0], 7, 7), vec![0, 255, 0, 255]);
    // diff_layer without diff_prev is a mistake.
    let e = c.err("render", json!({ "diff_layer": "dot" }));
    assert!(e.contains("diff_prev"), "{e}");
    // A layer-only diff of the still dot tints nothing.
    let d = c.call("render", json!({ "frame": 1, "scale": 1, "background": "transparent", "diff_prev": true, "diff_layer": "dot", "diff_opacity": 1.0 }));
    assert_eq!(at(&Client::images(&d)[0], 1, 1)[3], 0, "only the layer's own cel is compared, and it did not change");
}

// MARK: Warp

#[test]
fn warp_translates_with_exact_pixels_in_one_undo_step() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 4 }));
    c.ok("draw_pixels", json!({ "points": [[0, 1], [1, 1]], "color": "#ff0000" }));
    c.ok("draw_pixels", json!({ "points": [[2, 1]], "color": "#0000ff" }));
    let n = undo_steps(&mut c);
    let r = c.ok("warp", json!({ "grid": [2, 2], "offsets": [[2, 0], [2, 0], [2, 0], [2, 0]] }));
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["region"], json!({ "x": 0, "y": 0, "w": 8, "h": 4 }));
    assert_eq!(r["handles"], json!([[[0.0, 0.0], [8.0, 0.0]], [[0.0, 4.0], [8.0, 4.0]]]));
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(c.pixel(2, 1), "#ff0000ff");
    assert_eq!(c.pixel(3, 1), "#ff0000ff");
    assert_eq!(c.pixel(4, 1), "#0000ffff", "no blending, no new color");
    assert_eq!(alpha(&mut c, 0, 1), "00");
    c.ok("undo", json!({}));
    assert_eq!(c.pixel(0, 1), "#ff0000ff");
}

#[test]
fn warp_without_movement_is_a_no_op_and_only_the_selection_is_touched() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 8, "height": 4 }));
    c.ok("draw_pixels", json!({ "points": [[0, 1], [6, 1]], "color": "#ff0000" }));
    let n = undo_steps(&mut c);
    let r = c.ok("warp", json!({}));
    assert_eq!(r["changed"], false, "{r}");
    assert_eq!(undo_steps(&mut c), n, "no history step for a warp that moves nothing");
    // With a selection the grid covers the selection's bounds and nothing outside it changes.
    c.ok("select", json!({ "action": "rect", "x": 0, "y": 0, "w": 4, "h": 4 }));
    let n = undo_steps(&mut c);
    let r = c.ok("warp", json!({ "offsets": [[1, 0], [1, 0], [1, 0], [1, 0]] }));
    assert_eq!(r["region"], json!({ "x": 0, "y": 0, "w": 4, "h": 4 }), "{r}");
    assert_eq!(undo_steps(&mut c), n + 1);
    assert_eq!(c.pixel(1, 1), "#ff0000ff", "moved inside the selection");
    assert_eq!(alpha(&mut c, 0, 1), "00");
    assert_eq!(c.pixel(6, 1), "#ff0000ff", "outside the selection: untouched");
}

#[test]
fn warp_pins_smooth_preview_and_argument_errors() {
    let mut c = Client::start(&[]);
    c.ok("doc_new", json!({ "width": 16, "height": 16 }));
    c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [4, 4], "to": [11, 11], "color": "#00aa00", "filled": true }),
    );
    let n = undo_steps(&mut c);
    let before = c.ok("get_pixels", json!({ "format": "hex" }));
    // A preview shows the result and changes nothing.
    let args = json!({ "grid": [3, 3], "smooth": true, "samples": 2,
        "offsets": [[0,0],[0,0],[0,0],[0,0],[3,0],[0,0],[0,0],[0,0],[0,0]],
        "pins": [{ "x": 8.0, "y": 8.0, "dx": 2.0, "dy": 0.0, "radius": 6.0 }] });
    let mut preview = args.clone();
    preview["preview"] = json!(true);
    let p = c.call("warp", preview);
    assert_eq!(p["isError"], false, "{p}");
    assert_eq!(Client::images(&p).len(), 1);
    assert_eq!(undo_steps(&mut c), n);
    assert_eq!(c.ok("get_pixels", json!({ "format": "hex" })), before);
    let r = c.ok("warp", args);
    assert_eq!(r["changed"], true, "{r}");
    assert_eq!(r["pins"], 1);
    assert_eq!(r["grid"], json!([3, 3]));
    assert_ne!(c.ok("get_pixels", json!({ "format": "hex" })), before);
    // Errors.
    let e = c.err("warp", json!({ "grid": [3, 3], "offsets": [[0, 0]] }));
    assert!(e.contains("needs 9 offsets"), "{e}");
    let e = c.err("warp", json!({ "grid": [1, 2] }));
    assert!(e.contains("2-8"), "{e}");
    let e = c.err("warp", json!({ "grid": [9, 2] }));
    assert!(e.contains("2-8"), "{e}");
    let pins: Vec<Value> = (0..33).map(|i| json!({ "x": i, "y": 0, "dx": 1, "dy": 0, "radius": 2 })).collect();
    let e = c.err("warp", json!({ "pins": pins }));
    assert!(e.contains("at most 32"), "{e}");
}
