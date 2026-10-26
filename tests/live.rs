//! Live mode: the binary driven over stdio against an in-process mock of the Pixmith app's live server.

mod common;
use common::mock_app::{GAME_H, GAME_W, MockApp, live_dir};
use common::{Client, decode_png, write_png};
use serde_json::{Value, json};

fn start_with(extra: &[&str], env: &[(&str, &str)]) -> (Client, MockApp) {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApp::start(&Client::support(dir.path()));
    (Client::start_env(dir, extra, env), mock)
}

/// A client and a mock app that already has a blank 8x8 document ("doc-1").
fn start() -> (Client, MockApp) {
    let (c, mock) = start_with(&[], &[]);
    assert_eq!(mock.add_blank(8, 8, "hero.pxmt"), "doc-1");
    (c, mock)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const CLEAR: [u8; 4] = [0, 0, 0, 0];

#[test]
fn discovers_the_app_and_says_hello() {
    let (mut c, mock) = start();
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], true, "{s}");
    assert_eq!(s["mode"], "auto");
    assert_eq!(s["app"]["version"], "9.9.9");
    assert_eq!(s["app"]["front_end"], "mock");
    assert_eq!(s["app"]["protocol"], 1);
    assert_eq!(s["documents"][0]["doc"], "app:doc-1");
    assert_eq!(s["documents"][0]["name"], "hero.pxmt");
    assert_eq!(s["active"], "app:doc-1");
    assert_eq!(mock.with(|s| s.hellos), 1);
    // The connection is kept for later calls.
    c.ok("app_status", json!({}));
    assert_eq!(mock.with(|s| s.hellos), 1);
}

#[test]
fn a_wrong_token_is_refused_and_not_connected() {
    let dir = tempfile::tempdir().unwrap();
    let support = Client::support(dir.path());
    let live = live_dir(&support);
    std::fs::create_dir_all(&live).unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = json!({ "protocol": 1, "pid": std::process::id(), "port": port, "token": "nope", "app": "Pixmith",
                       "version": "1", "front_end": "swift", "platform": "macos", "started": 5 });
    std::fs::write(live.join(format!("{}.json", std::process::id())), body.to_string()).unwrap();
    // A server that does not know the token answers every first request with -32001.
    std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        while let Ok((s, _)) = listener.accept() {
            let mut line = String::new();
            let _ = std::io::BufReader::new(&s).read_line(&mut line);
            let _ = writeln!(&s, r#"{{"jsonrpc":"2.0","id":1,"error":{{"code":-32001,"message":"unauthorized"}}}}"#);
        }
    });
    let mut c = Client::start_in(dir, &[]);
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], false, "{s}");
    assert!(s["reason"].as_str().unwrap().contains("refused"), "{s}");
    // Headless keeps working.
    assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4 }))["doc"], "d1");
}

#[test]
fn stale_discovery_files_are_ignored_and_dead_ones_removed() {
    let dir = tempfile::tempdir().unwrap();
    let support = Client::support(dir.path());
    let mock = MockApp::start_as(&support, std::process::id(), 100);
    mock.add_blank(4, 4, "real.pxmt");
    let live = live_dir(&support);
    // Newer, but its process is gone.
    let dead = live.join("2147483000.json");
    std::fs::write(
        &dead,
        json!({ "protocol": 1, "pid": 2147483000u32, "port": mock.port, "token": "zzz", "started": 900 }).to_string(),
    )
    .unwrap();
    // Newer, its pid is alive (this test), but nothing listens on its port.
    let closed_port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let refused = live.join("1.json");
    std::fs::write(
        &refused,
        json!({ "protocol": 1, "pid": 1, "port": closed_port, "token": "yyy", "started": 800 }).to_string(),
    )
    .unwrap();
    std::fs::write(live.join("garbage.json"), "{ not json").unwrap();
    let mut c = Client::start_in(dir, &[]);
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], true, "{s}");
    assert_eq!(s["documents"][0]["name"], "real.pxmt");
    if cfg!(unix) {
        assert!(!dead.exists(), "the file of a dead process is deleted");
    }
    assert!(refused.exists(), "a refused port alone does not delete the file");
}

#[cfg(unix)]
#[test]
fn the_newest_app_is_chosen_and_app_connect_can_pick_by_pid() {
    let dir = tempfile::tempdir().unwrap();
    let support = Client::support(dir.path());
    let sleep = |_: u32| std::process::Command::new("sleep").arg("120").spawn().unwrap();
    let (mut s1, mut s2) = (sleep(1), sleep(2));
    let (p1, p2) = (s1.id(), s2.id());
    let old = MockApp::start_as(&support, p1, 1000);
    let new = MockApp::start_as(&support, p2, 2000);
    old.add_blank(4, 4, "old.pxmt");
    new.add_blank(4, 4, "new.pxmt");
    let mut c = Client::start_in(dir, &[]);
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["app"]["pid"], p2, "{s}");
    assert_eq!(s["documents"][0]["name"], "new.pxmt");
    let s = c.ok("app_connect", json!({ "pid": p1 }));
    assert_eq!(s["app"]["pid"], p1, "{s}");
    assert_eq!(s["documents"][0]["name"], "old.pxmt");
    let e = c.err("app_connect", json!({ "pid": 4242424 }));
    assert!(e.contains("no running Pixmith app with pid 4242424"), "{e}");
    let _ = (s1.kill(), s2.kill(), s1.wait(), s2.wait());
}

#[test]
fn support_dir_decides_where_the_app_is_looked_for() {
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApp::start(&Client::support(dir.path()));
    mock.add_blank(4, 4, "a.pxmt");
    let other = tempfile::tempdir().unwrap();
    let other_dir = other.path().to_string_lossy().into_owned();
    let mut c = Client::start_env(dir, &[], &[("PIXMITH_SUPPORT_DIR", &other_dir)]);
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], false, "{s}");
    assert!(s["reason"].as_str().unwrap().contains(&other_dir), "{s}");
}

#[test]
fn edits_land_in_the_app_as_one_labelled_undo_step() {
    let (mut c, mock) = start();
    let r0 = mock.revision("doc-1");
    // `doc` omitted: the app's active document.
    let out = c.ok(
        "draw_shape",
        json!({ "shape": "rectangle", "from": [1, 1], "to": [3, 3], "color": "#ff0000", "filled": true }),
    );
    assert_eq!(out["layer"].as_u64().unwrap_or(0), 0, "{out}");
    assert_eq!(mock.pixel("doc-1", 2, 2), RED);
    assert_eq!(mock.pixel("doc-1", 0, 0), CLEAR);
    assert_eq!(mock.undo_labels("doc-1"), vec!["AI: draw_shape rectangle"]);
    assert_eq!(mock.revision("doc-1"), r0 + 1);
    // Addressed explicitly, and the tool sees what the app has (including this edit).
    c.ok("draw_pixels", json!({ "doc": "app:doc-1", "points": [[6, 6]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 6, 6), RED);
    assert_eq!(mock.pixel("doc-1", 2, 2), RED);
    assert_eq!(mock.undo_labels("doc-1"), vec!["AI: draw_shape rectangle", "AI: draw_pixels"]);
    assert_eq!(c.pixel(2, 2), "#ff0000ff");
    // doc_info reads the app's document and names it with the app id.
    let info = c.ok("doc_info", json!({ "sections": ["layers", "history"] }));
    assert_eq!(info["doc"], "app:doc-1");
    assert_eq!(info["width"], 8);
    assert_eq!(info["dirty"], true);
    assert_eq!(info["history"]["managed_by"], "the Pixmith app");
}

#[test]
fn read_only_tools_and_no_op_edits_do_not_apply() {
    let (mut c, mock) = start();
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.applies(), 1);
    mock.clear_calls();
    c.ok("doc_info", json!({}));
    let r = c.call("render", json!({}));
    assert_eq!(r["isError"], false, "{r}");
    c.ok("get_pixels", json!({}));
    c.ok("get_pixel", json!({ "x": 0, "y": 0 }));
    c.ok("doc_list", json!({}));
    c.ok("help", json!({ "topic": "tools" }));
    assert_eq!(mock.applies(), 0, "{:?}", mock.calls());
    // An edit that changes nothing is not sent either.
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.applies(), 0, "{:?}", mock.calls());
    // A failing call changes nothing and is not sent.
    c.err("draw_pixels", json!({ "points": [[0, 0]], "color": "#12345" }));
    assert_eq!(mock.applies(), 0);
    // The unchanged document is not fetched again either: one `documents.get` served all of these.
    assert!(mock.count("documents.get") <= 1, "{:?}", mock.calls());
}

#[test]
fn a_stale_revision_refetches_and_retries() {
    let (mut c, mock) = start();
    mock.with(|s| s.race_next = 1);
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    // Two applies: the stale one and the retry; the document was fetched again in between.
    assert_eq!(mock.applies(), 2, "{:?}", mock.calls());
    assert_eq!(mock.count("documents.get"), 2, "{:?}", mock.calls());
    // The retry was made on the user's version: both the user's blue fill and the agent's pixel are there.
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    assert_eq!(mock.pixel("doc-1", 5, 5), [0, 0, 255, 255]);
    assert_eq!(mock.undo_labels("doc-1"), vec!["AI: draw_pixels"]);
}

#[test]
fn it_gives_up_after_three_stale_retries_and_changes_nothing() {
    let (mut c, mock) = start();
    mock.with(|s| s.race_next = 99);
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    assert!(e.contains("kept changing"), "{e}");
    assert_eq!(mock.applies(), 4, "one try and three retries: {:?}", mock.calls());
    assert!(mock.undo_labels("doc-1").is_empty());
    // Back to normal afterwards.
    mock.with(|s| s.race_next = 0);
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
}

#[test]
fn a_busy_app_is_retried_with_backoff() {
    let (mut c, mock) = start();
    mock.with(|s| s.busy_next = 2);
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    assert_eq!(mock.count("documents.apply"), 3, "{:?}", mock.calls());
    assert_eq!(mock.with(|s| s.applies.len()), 1);
    // An app that stays busy ends in a clear message and no change.
    mock.with(|s| s.busy_next = 100);
    let e = c.err("draw_pixels", json!({ "points": [[2, 2]], "color": "#ff0000" }));
    assert!(e.contains("busy") && e.contains("nothing was changed"), "{e}");
    assert_eq!(mock.pixel("doc-1", 2, 2), CLEAR);
}

#[test]
fn headless_and_app_documents_live_side_by_side() {
    let (mut c, mock) = start();
    let h = c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }));
    assert_eq!(h["doc"], "d1", "{h}");
    assert_eq!(mock.doc_count(), 1);
    let list = c.ok("doc_list", json!({}));
    let ids: Vec<&str> = list["documents"].as_array().unwrap().iter().map(|d| d["doc"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["app:doc-1", "d1"], "{list}");
    assert_eq!(list["documents"][0]["target"], "app");
    assert_eq!(list["documents"][1]["target"], "headless");
    assert_eq!(list["active"], "app:doc-1");
    assert_eq!(list["app"]["connected"], true);
    // A headless id stays headless; nothing reaches the app.
    mock.clear_calls();
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[0, 0]], "color": "#00ff00" }));
    assert!(mock.calls().is_empty(), "{:?}", mock.calls());
    // Without `doc`, the app wins.
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 0, 0), RED);
    let g = c.ok("get_pixel", json!({ "doc": "d1", "x": 0, "y": 0 }));
    assert_eq!(g["color"], "#00ff00ff");
    // Bare app ids work as well.
    assert_eq!(c.ok("doc_info", json!({ "doc": "doc-1", "sections": [] }))["doc"], "app:doc-1");
    let e = c.err("doc_info", json!({ "doc": "app:doc-9" }));
    assert!(e.contains("no document \"app:doc-9\"") && e.contains("app:doc-1"), "{e}");
}

#[test]
fn doc_new_creates_the_sprite_in_the_app() {
    let (mut c, mock) = start_with(&[], &[]);
    let r = c.ok("doc_new", json!({ "width": 6, "height": 5, "name": "slime" }));
    assert_eq!(r["doc"], "app:doc-1", "{r}");
    assert_eq!(r["target"], "app");
    assert_eq!(r["width"], 6);
    assert_eq!(r["name"], "slime");
    assert_eq!(mock.doc_count(), 1);
    assert_eq!(mock.applies(), 0, "a plain sprite is just created");
    // Extras (background, palette preset) are set up in the app as one step.
    let r = c.ok("doc_new", json!({ "width": 4, "height": 4, "background": "#ff0000", "palette_preset": "PICO-8" }));
    assert_eq!(r["doc"], "app:doc-2", "{r}");
    assert_eq!(mock.pixel("doc-2", 3, 3), RED);
    assert_eq!(mock.undo_labels("doc-2"), vec!["AI: doc_new"]);
    // New documents become the active one; tools find it without `doc`.
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#00ff00" }));
    assert_eq!(mock.pixel("doc-2", 0, 0), [0, 255, 0, 255]);
    assert_eq!(mock.pixel("doc-1", 0, 0), CLEAR);
    // Invalid arguments are refused before anything is created.
    let e = c.err("doc_new", json!({ "width": 0, "height": 4 }));
    assert!(e.contains("1-16384"), "{e}");
    assert_eq!(mock.doc_count(), 2);
    let e = c.err("doc_new", json!({ "width": 4, "height": 4, "target": "cloud" }));
    assert!(e.contains("target must be"), "{e}");
}

#[test]
fn doc_open_native_goes_through_the_app_and_other_formats_are_imported_into_a_new_tab() {
    let (mut c, mock) = start_with(&[], &[]);
    // A .pxmt file made headless.
    c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }));
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[1, 1]], "color": "#ff0000" }));
    let pxmt = c.p("art.pxmt");
    c.ok("doc_save", json!({ "doc": "d1", "path": &pxmt }));
    let png = c.p("art.png");
    c.ok("export", json!({ "doc": "d1", "format": "png", "path": &png }));
    let r = c.ok("doc_open", json!({ "path": &pxmt }));
    assert_eq!(r["doc"], "app:doc-1", "{r}");
    assert_eq!(mock.count("documents.open"), 1);
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    // The same file again is the already open tab.
    assert_eq!(c.ok("doc_open", json!({ "path": &pxmt }))["doc"], "app:doc-1");
    assert_eq!(mock.doc_count(), 1);
    // A PNG is imported here and arrives as a new tab.
    let r = c.ok("doc_open", json!({ "path": &png }));
    assert_eq!(r["doc"], "app:doc-2", "{r}");
    assert!(r["imported_from"].as_str().unwrap().ends_with("art.png"));
    assert_eq!(mock.pixel("doc-2", 1, 1), RED);
    assert_eq!(mock.undo_labels("doc-2"), vec!["AI: doc_open"]);
    let e = c.err("doc_open", json!({ "path": "missing.pxmt" }));
    assert!(e.contains("missing.pxmt"), "{e}");
}

#[test]
fn doc_save_and_doc_close_map_to_the_app() {
    let (mut c, mock) = start();
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    let e = c.err("doc_save", json!({}));
    assert!(e.contains("has no file yet"), "{e}");
    let e = c.err("doc_save", json!({ "path": c.p("x.png") }));
    assert!(e.contains("use the export tool"), "{e}");
    let out = c.ok("doc_save", json!({ "path": c.p("hero.pxmt") }));
    assert_eq!(out["dirty"], false, "{out}");
    assert!(c.path("hero.pxmt").exists());
    assert_eq!(mock.count("documents.save"), 1);
    // The saved file is the app's document.
    assert_eq!(pixmith_core::formats::pxmt::open_file(c.path("hero.pxmt")).unwrap().width, 8);
    // A copy leaves the document's path and state alone.
    c.ok("draw_pixels", json!({ "points": [[2, 2]], "color": "#ff0000" }));
    let out = c.ok("doc_save", json!({ "path": c.p("copy.pxmt"), "copy": true }));
    assert_eq!(out["copy"], true);
    assert!(c.path("copy.pxmt").exists());
    assert_eq!(mock.count("documents.save"), 1);
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["dirty"], true);
    // Closing refuses unsaved changes (the app's -32011), then closes on request.
    let e = c.err("doc_close", json!({}));
    assert!(e.contains("unsaved changes") && e.contains("discard_changes"), "{e}");
    let out = c.ok("doc_close", json!({ "discard_changes": true }));
    assert_eq!(out["closed"], "app:doc-1");
    assert_eq!(mock.doc_count(), 0);
    let e = c.err("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert!(e.contains("no document is open in the Pixmith app"), "{e}");
}

#[test]
fn layer_and_frame_follow_the_edit_and_doc_set_active_moves_the_view() {
    let (mut c, mock) = start();
    c.ok("layer", json!({ "action": "add", "name": "top" }));
    // The new layer is current, in the same apply.
    assert_eq!(mock.view("doc-1"), (1, 0));
    assert_eq!(
        mock.with(|s| s.applies.last().map(|a| (a.layer, a.label.clone()))),
        Some((Some(1), "layer add".into()))
    );
    // The tool defaults to the app's current layer: draw on layer 1.
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    let info = c.ok("doc_info", json!({ "sections": ["layers"] }));
    assert_eq!(info["active_layer"], 1);
    let px = c.ok("get_pixels", json!({ "layer": 1, "region": { "x": 1, "y": 1, "w": 1, "h": 1 }, "format": "hex" }));
    assert!(px.to_string().contains("ff0000"), "{px}");
    // Aimed at another layer, the edit moves the app's current layer there.
    c.ok("draw_pixels", json!({ "layer": 0, "points": [[3, 3]], "color": "#00ff00" }));
    assert_eq!(mock.view("doc-1"), (0, 0));
    // The user switches layers in the app; the tool follows without re-fetching the sprite.
    mock.with(|s| s.docs[0].layer = 1);
    let gets = mock.count("documents.get");
    assert_eq!(c.ok("doc_info", json!({ "sections": [] }))["active_layer"], 1);
    assert_eq!(mock.count("documents.get"), gets);
    // doc_set_active changes only the view.
    mock.clear_calls();
    let out = c.ok("doc_set_active", json!({ "layer": "Layer 1" }));
    assert_eq!(out["doc"], "app:doc-1", "{out}");
    assert_eq!(mock.view("doc-1"), (0, 0));
    assert_eq!(mock.applies(), 0);
    assert_eq!(mock.count("view.set"), 1);
    // With neither layer nor frame it activates the document's tab.
    let second = c.ok("doc_new", json!({ "width": 2, "height": 2 }))["doc"].as_str().unwrap().to_string();
    assert_eq!(second, "app:doc-2");
    c.ok("doc_set_active", json!({ "doc": "app:doc-1" }));
    assert_eq!(mock.with(|s| s.active.clone()), Some("doc-1".into()));
}

#[test]
fn a_selection_survives_between_calls_until_the_user_changes_the_document() {
    let (mut c, mock) = start();
    c.ok("select", json!({ "action": "rect", "x": 0, "y": 0, "w": 4, "h": 8 }));
    assert_eq!(mock.applies(), 0, "a selection is not part of the document");
    c.ok("fill", json!({ "x": 0, "y": 0, "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    assert_eq!(mock.pixel("doc-1", 6, 1), CLEAR, "the fill was clipped to the selection");
    // The selection is still there for the next call.
    let sel = c.ok("doc_info", json!({ "sections": ["selection"] }));
    assert_eq!(sel["selection"]["active"], true, "{sel}");
    // After a change in the app, the next call starts from the app's document.
    mock.user_fill("doc-1", 0x0000ffff);
    let sel = c.ok("doc_info", json!({ "sections": ["selection"] }));
    assert_eq!(sel["selection"]["active"], false, "{sel}");
}

#[test]
fn undo_goes_to_the_app_when_it_can_and_says_so_when_it_cannot() {
    let (mut c, mock) = start();
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    // This app has no documents.undo (the protocol does not have it yet).
    let e = c.err("undo", json!({}));
    assert!(e.contains("not supported in live mode") && e.contains("Cmd/Ctrl+Z"), "{e}");
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    for t in ["history", "history_goto"] {
        let e = c.err(t, if t == "history" { json!({}) } else { json!({ "position": 0 }) });
        assert!(e.contains("not available in live mode") && e.contains("AI: <tool>"), "{t}: {e}");
    }
    // An app that offers it: undo and redo are its own.
    mock.with(|s| s.undo_supported = true);
    let out = c.ok("undo", json!({}));
    assert_eq!(out["undone"], 1, "{out}");
    assert_eq!(mock.pixel("doc-1", 1, 1), CLEAR);
    let out = c.ok("undo", json!({}));
    assert_eq!(out["undone"], 0);
    let out = c.ok("redo", json!({}));
    assert_eq!(out["redone"], 1);
    assert_eq!(mock.pixel("doc-1", 1, 1), RED);
    // Transactions are not needed: every call is a step, and begin/end are harmless.
    let t = c.ok("transaction", json!({ "action": "begin", "label": "x" }));
    assert!(t["note"].as_str().unwrap().contains("not used in live mode"), "{t}");
    c.ok("transaction", json!({ "action": "end" }));
}

#[test]
fn disconnect_and_connect_switch_at_runtime() {
    let (mut c, mock) = start();
    let s = c.ok("app_disconnect", json!({}));
    assert_eq!(s["connected"], false, "{s}");
    assert!(s["reason"].as_str().unwrap().contains("app_connect"), "{s}");
    // Headless now, and it does not reconnect by itself.
    assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4 }))["doc"], "d1");
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 0, 0), CLEAR);
    assert_eq!(c.ok("doc_list", json!({}))["app"]["connected"], false);
    let e = c.err("get_pixel", json!({ "doc": "app:doc-1", "x": 0, "y": 0 }));
    assert!(e.contains("app_connect"), "{e}");
    let s = c.ok("app_connect", json!({}));
    assert_eq!(s["connected"], true, "{s}");
    assert_eq!(s["headless_documents"], json!(["d1"]));
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 0, 0), RED);
}

#[test]
fn no_app_means_headless_exactly_as_before() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::start_in(dir, &[]);
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], false);
    assert_eq!(s["mode"], "auto");
    assert!(s["reason"].as_str().unwrap().contains("no running Pixmith app"), "{s}");
    let r = c.ok("doc_new", json!({ "width": 8, "height": 8 }));
    assert_eq!(r["doc"], "d1");
    assert!(r.get("target").is_none(), "never connected: no live clutter in the result: {r}");
    let r = c.ok("draw_shape", json!({ "shape": "line", "from": [0, 0], "to": [7, 7], "color": "#ff0000" }));
    assert!(r.get("target").is_none(), "{r}");
    assert_eq!(c.pixel(3, 3), "#ff0000ff");
    c.ok("undo", json!({}));
    assert_eq!(c.pixel(3, 3), "#00000000");
    let list = c.ok("doc_list", json!({}));
    assert_eq!(list["documents"][0]["doc"], "d1");
    assert!(list["documents"][0].get("target").is_none(), "no live clutter without an app: {list}");
    let e = c.err("doc_info", json!({ "doc": "app:doc-1" }));
    assert!(e.contains("not connected"), "{e}");
    let e = c.err("doc_new", json!({ "width": 4, "height": 4, "target": "app" }));
    assert!(e.contains("not connected"), "{e}");
}

#[test]
fn the_app_can_start_or_quit_while_the_server_runs() {
    let dir = tempfile::tempdir().unwrap();
    let support = Client::support(dir.path());
    let mut c = Client::start_in(dir, &[]);
    // No app yet: headless.
    assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4 }))["doc"], "d1");
    // The app starts: found on the next call, and it wins for calls without `doc`.
    let mock = MockApp::start(&support);
    mock.add_blank(8, 8, "late.pxmt");
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 0, 0), RED);
    // The app quits: this server was connected, so calls without `doc` no longer fall back to the headless document
    // silently (see the lost-connection test); explicit ids still work, explicit app ids say what happened.
    mock.quit();
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "#00ff00" }));
    assert!(e.contains("was connected but is not reachable"), "{e}");
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[1, 1]], "color": "#00ff00" }));
    assert_eq!(c.ok("get_pixel", json!({ "doc": "d1", "x": 1, "y": 1 }))["color"], "#00ff00ff");
    let e = c.err("get_pixel", json!({ "doc": "app:doc-1", "x": 0, "y": 0 }));
    assert!(e.contains("not connected") || e.contains("connection"), "{e}");
    // A new instance is picked up again.
    let again = MockApp::start_as(&support, std::process::id(), 1_800_000_000);
    again.add_blank(8, 8, "second.pxmt");
    c.ok("draw_pixels", json!({ "points": [[2, 2]], "color": "#0000ff" }));
    assert_eq!(again.pixel("doc-1", 2, 2), [0, 0, 255, 255]);
}

#[test]
fn live_off_never_talks_to_the_app_and_on_requires_it() {
    for (extra, env) in [(vec!["--live", "off"], vec![]), (vec![], vec![("PIXMITH_LIVE", "off")])] {
        let (mut c, mock) = start_with(&extra, &env);
        mock.add_blank(4, 4, "a.pxmt");
        let s = c.ok("app_status", json!({}));
        assert_eq!(s["connected"], false, "{s}");
        assert_eq!(s["mode"], "off");
        assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4 }))["doc"], "d1");
        let e = c.err("doc_info", json!({ "doc": "app:doc-1" }));
        assert!(e.contains("live mode is off"), "{e}");
        let e = c.err("app_connect", json!({}));
        assert!(e.contains("live mode is off"), "{e}");
        assert_eq!(mock.with(|s| s.hellos), 0, "{extra:?}");
    }
    // `on` without an app: a clear error instead of a silent headless document.
    let dir = tempfile::tempdir().unwrap();
    let mut c = Client::start_in(dir, &["--live", "on"]);
    let e = c.err("doc_new", json!({ "width": 4, "height": 4 }));
    assert!(e.contains("live mode is on") && e.contains("no Pixmith app"), "{e}");
    let e = c.err("draw_pixels", json!({ "points": [[0, 0]], "color": "#fff" }));
    assert!(e.contains("live mode is on"), "{e}");
    // Headless stays reachable on request.
    assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }))["doc"], "d1");
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[0, 0]], "color": "#fff" }));
    assert_eq!(c.ok("app_status", json!({}))["mode"], "on");
    // With an app, `on` behaves like `auto`.
    let dir = tempfile::tempdir().unwrap();
    let mock = MockApp::start(&Client::support(dir.path()));
    let mut c = Client::start_in(dir, &["--live=on"]);
    assert_eq!(c.ok("doc_new", json!({ "width": 4, "height": 4 }))["doc"], "app:doc-1");
    assert_eq!(mock.doc_count(), 1);
}

#[test]
fn a_bad_live_mode_is_refused_at_startup() {
    let dir = tempfile::tempdir().unwrap();
    let run = |args: &[&str], env: &[(&str, &str)]| {
        let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_pixmith-mcp"));
        cmd.args(args).current_dir(dir.path()).stdin(std::process::Stdio::null());
        for (k, v) in env {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    };
    let o = run(&["--live", "sometimes"], &[]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("--live"), "{}", String::from_utf8_lossy(&o.stderr));
    let o = run(&[], &[("PIXMITH_LIVE", "maybe")]);
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("PIXMITH_LIVE"));
    // print-config carries the flag into the client configuration.
    let o = run(&["print-config", "json", "--live", "on"], &[]);
    assert!(o.status.success());
    let cfg: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(cfg["mcpServers"]["pixmith"]["args"], json!(["--live", "on"]));
    let o = run(&["print-config", "json", "--live", "x"], &[]);
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn resources_and_the_catalogue_know_about_the_app() {
    let (mut c, _mock) = start();
    let r = c.request("tools/list", json!({}));
    let names: Vec<&str> =
        r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    for n in ["app_status", "app_connect", "app_disconnect", "game_screenshot"] {
        assert!(names.contains(&n), "{n}");
    }
    let list = c.request("resources/list", json!({}));
    let uris: Vec<&str> =
        list["result"]["resources"].as_array().unwrap().iter().map(|r| r["uri"].as_str().unwrap()).collect();
    assert!(uris.contains(&"pixmith://documents/app:doc-1/info"), "{uris:?}");
    let img = c.request("resources/read", json!({ "uri": "pixmith://documents/app:doc-1/render.png" }));
    assert!(img["result"]["contents"][0]["blob"].is_string(), "{img}");
    let info = c.request("resources/read", json!({ "uri": "pixmith://documents/app:doc-1/info" }));
    assert!(info["result"]["contents"][0]["text"].as_str().unwrap().contains("app:doc-1"), "{info}");
    let init_guide = c.ok("help", json!({}));
    assert!(init_guide.as_str().unwrap().contains("Live mode"), "the guide explains live mode");
}

#[test]
fn render_shows_what_the_app_has() {
    let (mut c, mock) = start();
    mock.user_fill("doc-1", 0xff0000ff);
    let r = c.call("render", json!({ "scale": 1 }));
    let png = base64_png(&r);
    let (w, h, rgba) = decode_png(&png);
    assert_eq!((w, h), (8, 8));
    assert_eq!(&rgba[..4], &RED);
}

#[test]
fn game_screenshot_returns_the_game_as_image_content() {
    let (mut c, mock) = start();
    mock.with(|s| s.game = true);
    let r = c.call("game_screenshot", json!({}));
    assert_eq!(r["isError"], json!(false), "{r}");
    let info: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!((info["w"].as_u64(), info["h"].as_u64()), (Some(GAME_W as u64), Some(GAME_H as u64)), "{info}");
    assert_eq!(info["client"], "pixmith-godot");
    let imgs = Client::images(&r);
    assert_eq!(imgs.len(), 1);
    let (w, h, rgba) = &imgs[0];
    assert_eq!((*w, *h), (GAME_W, GAME_H));
    assert_eq!(&rgba[..4], &RED);
    assert_eq!(&rgba[(GAME_W as usize - 1) * 4..GAME_W as usize * 4], &[0, 0, 255, 255]);
    assert_eq!(mock.count("game.capture"), 1);
}

#[test]
fn game_screenshot_scales_with_nearest_neighbour_and_caps_the_long_side() {
    let (mut c, mock) = start();
    mock.with(|s| s.game = true);
    let r = c.call("game_screenshot", json!({ "scale": 3, "timeout_ms": 500 }));
    let info: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!((info["w"].as_u64(), info["h"].as_u64()), (Some(12), Some(6)), "{info}");
    let (w, h, rgba) = &Client::images(&r)[0];
    assert_eq!((*w, *h), (12, 6));
    assert_eq!(&rgba[5 * 4..6 * 4], &RED, "pixel 5 is still the left half");
    assert_eq!(&rgba[6 * 4..7 * 4], &[0, 0, 255, 255]);
    assert!(c.err("game_screenshot", json!({ "scale": 9 })).contains("1-8"));
    assert!(c.err("game_screenshot", json!({ "scale": 0 })).contains("1-8"));
}

#[test]
fn game_screenshot_without_a_game_is_a_friendly_tool_error() {
    let (mut c, mock) = start();
    let e = c.err("game_screenshot", json!({}));
    assert!(e.contains("no running game with the Pixmith Live Link addon"), "{e}");
    assert!(e.contains("start the game from Godot (debug) with the addon enabled"), "{e}");
    // The app stays connected and works.
    assert_eq!(c.ok("app_status", json!({}))["connected"], true);
    mock.with(|s| s.game = true);
    assert_eq!(c.call("game_screenshot", json!({}))["isError"], json!(false));
}

#[test]
fn game_screenshot_asks_for_a_newer_app_when_it_does_not_know_game_capture() {
    let (mut c, mock) = start();
    mock.with(|s| s.no_game_view = true);
    let e = c.err("game_screenshot", json!({}));
    assert!(e.contains("0.5.1"), "{e}");
}

#[test]
fn game_screenshot_headless_is_a_tool_error() {
    let mut c = Client::start(&[]);
    let e = c.err("game_screenshot", json!({}));
    assert!(e.contains("Pixmith Live Link addon"), "{e}");
    assert!(e.contains("Pixmith app"), "{e}");
    let mut off = Client::start(&["--live", "off"]);
    let e = off.err("game_screenshot", json!({}));
    assert!(e.contains("live mode is off"), "{e}");
}

fn base64_png(result: &Value) -> Vec<u8> {
    use base64::Engine as _;
    let img = result["content"].as_array().unwrap().iter().find(|c| c["type"] == "image").expect("image");
    base64::engine::general_purpose::STANDARD.decode(img["data"].as_str().unwrap()).unwrap()
}

// MARK: Lost connections, replace, parallel calls, headless housekeeping

const PROBLEM: &str = "was connected but is not reachable";

#[test]
fn a_lost_connection_is_an_error_not_a_silent_headless_document() {
    let (mut c, mock) = start();
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    assert_eq!(mock.pixel("doc-1", 0, 0), RED);
    mock.quit();

    // Creating and editing must not quietly go headless.
    let e = c.err("doc_new", json!({ "width": 4, "height": 4 }));
    assert!(e.contains(PROBLEM) && e.contains("target: \"headless\"") && e.contains("app_disconnect"), "{e}");
    assert!(e.contains("lost connection to the Pixmith app") && e.contains("app_connect"), "{e}");
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "#00ff00" }));
    assert!(e.contains(PROBLEM), "{e}");
    let e = c.err("doc_info", json!({}));
    assert!(e.contains(PROBLEM), "{e}");
    // What broke is part of the message.
    assert!(e.contains("the Pixmith app closed") || e.contains("cannot talk") || e.contains("no answer from"), "{e}");
    // Nothing headless was created behind the user's back.
    let list = c.ok("doc_list", json!({}));
    assert_eq!(list["documents"], json!([]), "{list}");
    assert_eq!(list["app"]["connected"], false);
    assert!(list["app"]["reason"].as_str().unwrap().contains(PROBLEM), "{list}");
    let s = c.ok("app_status", json!({}));
    assert_eq!(s["connected"], false);
    assert!(s["note"].as_str().unwrap().contains("silently"), "{s}");

    // Headless on purpose still works, and says it is headless.
    let h = c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }));
    assert_eq!(h["doc"], "d1", "{h}");
    assert_eq!(h["target"], "headless", "{h}");
    c.ok("draw_pixels", json!({ "doc": "d1", "points": [[1, 1]], "color": "#00ff00" }));
    // ...but still not by default.
    let e = c.err("draw_pixels", json!({ "points": [[2, 2]], "color": "#00ff00" }));
    assert!(e.contains(PROBLEM), "{e}");

    // app_disconnect is the explicit way to work headless: no more errors, results say headless.
    c.ok("app_disconnect", json!({}));
    let r = c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    assert_eq!(r["doc"], "d2");
    assert_eq!(r["target"], "headless");
    c.ok("draw_pixels", json!({ "doc": "d2", "points": [[0, 0]], "color": "#fff" }));
    // The app is still gone: app_connect says so (and does not turn the disconnect off by itself).
    let e = c.err("app_connect", json!({}));
    assert!(e.contains("no running Pixmith app"), "{e}");
}

#[test]
fn a_restarted_app_is_picked_up_after_the_connection_was_lost() {
    let dir = tempfile::tempdir().unwrap();
    let support = Client::support(dir.path());
    let first = MockApp::start_as(&support, std::process::id(), 100);
    first.add_blank(8, 8, "one.pxmt");
    let mut c = Client::start_in(dir, &[]);
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    first.quit();
    let e = c.err("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    assert!(e.contains(PROBLEM), "{e}");
    // The app comes back: the very next call reconnects (once) and works.
    let second = MockApp::start_as(&support, std::process::id(), 200);
    second.add_blank(8, 8, "two.pxmt");
    c.ok("draw_pixels", json!({ "points": [[1, 1]], "color": "#ff0000" }));
    assert_eq!(second.pixel("doc-1", 1, 1), RED);
    let r = c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    assert_eq!(r["doc"], "app:doc-2", "{r}");
}

#[test]
fn a_broken_socket_is_reconnected_once_before_giving_up() {
    // The app is still there (same discovery file) but our socket is dead: the probe before a create reconnects.
    let (mut c, mock) = start();
    c.ok("draw_pixels", json!({ "points": [[0, 0]], "color": "#ff0000" }));
    let hellos = mock.with(|s| s.hellos);
    // Drop every connection from the app's side while its listener stays up.
    mock.with(|s| s.drop_connections = true);
    std::thread::sleep(std::time::Duration::from_millis(400));
    mock.with(|s| s.drop_connections = false);
    let r = c.ok("doc_new", json!({ "width": 4, "height": 4 }));
    assert_eq!(r["doc"], "app:doc-2", "{r}");
    assert_eq!(mock.with(|s| s.hellos), hellos + 1, "reconnected exactly once");
}

#[test]
fn a_call_that_lands_on_a_headless_document_while_connected_says_so() {
    let (mut c, _mock) = start_with(&[], &[]);
    // Connected, the app has no document, so a call without `doc` can only use the headless one.
    c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }));
    let r = c.ok("get_pixel", json!({ "x": 0, "y": 0 }));
    assert_eq!(r["target"], "headless", "{r}");
    // An explicit id is deliberate and stays quiet.
    let r = c.ok("get_pixel", json!({ "doc": "d1", "x": 0, "y": 0 }));
    assert!(r.get("target").is_none(), "{r}");
}

#[test]
fn layer_import_png_replace_is_one_labelled_step_in_the_app() {
    let (mut c, mock) = start();
    write_png(&c.path("red.png"), 2, 2, &[RED; 4].concat());
    mock.clear_calls();
    let r = c.ok("layer", json!({ "action": "import_png", "path": "red.png", "replace": true, "x": 2, "y": 3 }));
    assert_eq!(r["layers"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(mock.applies(), 1);
    assert_eq!(mock.undo_labels("doc-1"), vec!["AI: layer import_png"]);
    for (x, y) in [(2, 3), (3, 3), (2, 4), (3, 4)] {
        assert_eq!(mock.pixel("doc-1", x, y), RED, "({x}, {y})");
    }
    assert_eq!(mock.pixel("doc-1", 1, 3), CLEAR);
    // Replacing again swaps the pixels: still one layer, a second step.
    write_png(&c.path("blue.png"), 1, 1, &[0, 0, 255, 255]);
    c.ok("layer", json!({ "action": "import_png", "path": "blue.png", "replace": true }));
    assert_eq!(mock.pixel("doc-1", 0, 0), [0, 0, 255, 255]);
    assert_eq!(mock.pixel("doc-1", 2, 3), CLEAR, "the old pixels are gone");
    assert_eq!(mock.undo_labels("doc-1").len(), 2);
    assert_eq!(c.ok("doc_info", json!({ "sections": ["layers"] }))["layers"].as_array().unwrap().len(), 1);
}

#[test]
fn parallel_doc_new_calls_run_one_at_a_time_and_none_is_lost() {
    let (mut c, mock) = start();
    let calls: Vec<(&str, Value)> = (0..8)
        .map(|i| ("doc_new", json!({ "width": 4 + i, "height": 4, "name": format!("p{i}"), "background": "#ff0000" })))
        .collect();
    let results = c.call_many(&calls);
    assert_eq!(mock.doc_count(), 9, "doc-1 plus eight new documents");
    let mut ids = Vec::new();
    for (i, r) in results.iter().enumerate() {
        assert_eq!(r["isError"], json!(false), "{r}");
        let v: Value = serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(v["name"], format!("p{i}"), "each answer belongs to its own request: {v}");
        assert_eq!(v["width"], 4 + i as u64, "{v}");
        assert_eq!(v["target"], "app");
        let id = v["doc"].as_str().unwrap().to_string();
        assert_eq!(mock.pixel(id.strip_prefix("app:").unwrap(), 0, 0), RED, "{id}");
        ids.push(id);
    }
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), 8, "{ids:?}");
    // Mixed parallel work (create, edit, close) keeps every request whole too.
    let target = ids[0].clone();
    let mixed = c.call_many(&[
        ("doc_new", json!({ "width": 5, "height": 5 })),
        ("draw_pixels", json!({ "doc": target, "points": [[1, 1]], "color": "#00ff00" })),
        ("doc_close", json!({ "doc": ids[1], "discard_changes": true })),
        ("doc_new", json!({ "width": 6, "height": 6 })),
    ]);
    assert!(mixed.iter().all(|r| r["isError"] == json!(false)), "{mixed:?}");
    assert_eq!(mock.doc_count(), 10);
}

#[test]
fn closing_all_headless_documents_never_touches_the_app() {
    let (mut c, mock) = start();
    for _ in 0..2 {
        c.ok("doc_new", json!({ "width": 4, "height": 4, "target": "headless" }));
    }
    mock.clear_calls();
    let r = c.ok("doc_close", json!({ "all_headless": true }));
    assert_eq!(r["closed"], json!(["d1", "d2"]), "{r}");
    assert!(mock.calls().is_empty(), "{:?}", mock.calls());
    assert_eq!(mock.doc_count(), 1);
    assert_eq!(c.ok("doc_list", json!({}))["documents"][0]["doc"], "app:doc-1");
}

#[test]
fn the_headless_limit_applies_next_to_the_app_and_ignores_app_documents() {
    let (mut c, mock) = start();
    let mut last = Value::Null;
    for _ in 0..33 {
        last = c.ok("doc_new", json!({ "width": 2, "height": 2, "target": "headless" }));
    }
    assert_eq!(last["closed_headless"], json!(["d1"]), "{last}");
    assert!(last["note"].as_str().unwrap().contains("oldest unmodified"), "{last}");
    // App documents are not counted and not closed.
    assert_eq!(mock.doc_count(), 1);
    let headless = c.ok("doc_list", json!({}))["documents"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["target"] == "headless")
        .count();
    assert_eq!(headless, 32);
    let r = c.ok("doc_new", json!({ "width": 2, "height": 2 }));
    assert_eq!(r["doc"], "app:doc-2");
    assert!(r.get("note").is_none(), "{r}");
}

fn tiny_png(c: &Client, name: &str) -> std::path::PathBuf {
    let p = c.path(name);
    common::write_png(&p, 2, 2, &[255, 0, 0, 255].repeat(4));
    p
}

#[test]
fn doc_save_replace_closes_a_clean_tab_that_holds_the_file() {
    let (mut c, mock) = start();
    let file = c.path("hero.pxmt");
    std::fs::write(&file, b"x").unwrap();
    let canon = std::fs::canonicalize(&file).unwrap().to_string_lossy().into_owned();
    // doc-2 holds the file; doc-1 wants to save there.
    let two = mock.with(|s| s.add(pixmith_core::Sprite::new(8, 8), "old.pxmt", Some(canon.clone())));
    assert_eq!(two, "doc-2");
    let e = c.err("doc_save", json!({ "doc": "app:doc-1", "path": file.to_string_lossy() }));
    assert!(e.contains("replace: true") && e.contains("app:doc-2"), "{e}");
    assert_eq!(mock.doc_count(), 2);
    // A tab with unsaved changes is never closed.
    mock.user_fill("doc-2", 0xff0000ff);
    let e = c.err("doc_save", json!({ "doc": "app:doc-1", "path": file.to_string_lossy(), "replace": true }));
    assert!(e.contains("unsaved changes") && e.contains("nothing was saved"), "{e}");
    assert_eq!(mock.doc_count(), 2);
    // Clean again: replace closes it and saves.
    mock.with(|s| s.docs.iter_mut().find(|d| d.id == "doc-2").unwrap().dirty = false);
    let r = c.ok("doc_save", json!({ "doc": "app:doc-1", "path": file.to_string_lossy(), "replace": true }));
    assert_eq!(r["closed_tabs"], json!(["app:doc-2"]), "{r}");
    assert_eq!(mock.doc_count(), 1);
}

#[test]
fn a_failed_import_does_not_leave_a_half_built_tab() {
    let (mut c, mock) = start();
    let png = tiny_png(&c, "sheet.png");
    mock.with(|s| s.apply_fail_next = 1);
    let e = c.err("doc_open", json!({ "path": png.to_string_lossy(), "target": "app" }));
    assert!(e.contains("could not apply") && e.contains("was closed again"), "{e}");
    assert_eq!(mock.doc_count(), 1, "the tab made for the import is gone");
    // The next open works.
    let r = c.ok("doc_open", json!({ "path": png.to_string_lossy(), "target": "app" }));
    assert_eq!(r["target"], "app", "{r}");
    assert_eq!(mock.doc_count(), 2);
}

#[test]
fn an_imported_app_document_is_saved_next_to_its_source() {
    let (mut c, mock) = start();
    let png = tiny_png(&c, "walk.png");
    let r = c.ok("doc_open", json!({ "path": png.to_string_lossy(), "target": "app" }));
    let id = r["doc"].as_str().unwrap().to_string();
    let s = c.ok("doc_save", json!({ "doc": id }));
    let saved = s["saved"].as_str().unwrap();
    assert!(saved.ends_with("walk.pxmt"), "{s}");
    assert_eq!(std::path::Path::new(saved).parent().unwrap(), std::fs::canonicalize(c.path("")).unwrap(), "{s}");
    assert!(s["note"].as_str().unwrap().contains("next to the imported file"), "{s}");
    assert_eq!(mock.doc_count(), 2);
}
