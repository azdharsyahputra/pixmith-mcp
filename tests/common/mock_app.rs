//! An in-process stand-in for the Pixmith app's live server, speaking the Live protocol
//! (`docs/LIVE_PROTOCOL.md` in the Pixmith repository). Documents are real `pixmith-core` editors, so
//! `documents.apply` really swaps sprites; the mock also has revisions, `-32009` / `-32010` / `-32011`, a
//! discovery file, and knobs to make things go wrong.
#![allow(dead_code)]

use base64::Engine as _;
use pixmith_core::formats::pxmt;
use pixmith_core::{Editor, Sprite};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct MockDoc {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub editor: Editor,
    pub revision: u64,
    pub dirty: bool,
    pub layer: u32,
    pub frame: u32,
    /// What the app's undo stack holds: the label and the sprite before the step.
    pub undo: Vec<(String, Sprite)>,
    pub redo: Vec<(String, Sprite)>,
}

pub struct ApplyRecord {
    pub id: String,
    pub label: String,
    pub base_revision: u64,
    pub layer: Option<u64>,
    pub frame: Option<u64>,
}

#[derive(Default)]
pub struct State {
    pub docs: Vec<MockDoc>,
    pub active: Option<String>,
    next: u32,
    /// Every request after hello, in order: the method name.
    pub calls: Vec<String>,
    pub applies: Vec<ApplyRecord>,
    /// Answer the next n `documents.apply` with -32010.
    pub busy_next: u32,
    /// Before each of the next n `documents.apply`, the "user" paints the whole first layer of the document
    /// with this color, so the revision the client holds is stale.
    pub race_next: u32,
    pub race_color: u32,
    pub undo_supported: bool,
    pub clients: u32,
    pub hellos: u32,
    /// A running game with the Live Link addon: `game.capture` returns `GAME_W` x `GAME_H` (left half red,
    /// right half blue). Without one it fails with -32012.
    pub game: bool,
    /// `game.capture` is unknown to this "older" app (-32601).
    pub no_game_view: bool,
    /// Fail the next n `documents.apply` with a generic error (-32000), like an app that choked on a big document.
    pub apply_fail_next: u32,
    /// An older app: `documents.new` knows only width and height (a `pxmt` request is -32602).
    pub no_pxmt_new: bool,
    /// Close every client connection (the listener stays up), like an app that dropped the socket.
    pub drop_connections: bool,
}

pub const GAME_W: u32 = 4;
pub const GAME_H: u32 = 2;

/// The frame the mock game sends: a tiny PNG, red on the left, blue on the right.
pub fn game_png() -> Vec<u8> {
    let mut px = Vec::new();
    for _ in 0..GAME_H {
        for x in 0..GAME_W {
            px.extend_from_slice(if x < GAME_W / 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 255] });
        }
    }
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, GAME_W, GAME_H);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header().unwrap().write_image_data(&px).unwrap();
    }
    out
}

impl State {
    fn doc_json(&self, d: &MockDoc) -> Value {
        let raw = &d.editor as *const Editor as *mut Editor;
        json!({
            "id": d.id, "name": d.name, "path": d.path,
            "width": d.editor.sprite.width, "height": d.editor.sprite.height,
            "frames": unsafe { pixmith_core::ffi::document::pixmith_frame_count(raw) },
            "layers": unsafe { pixmith_core::ffi::document::pixmith_layer_count(raw) },
            "revision": d.revision, "dirty": d.dirty, "active": self.active.as_deref() == Some(d.id.as_str()),
            "layer": d.layer, "frame": d.frame,
        })
    }

    pub fn add(&mut self, sprite: Sprite, name: &str, path: Option<String>) -> String {
        self.next += 1;
        let id = format!("doc-{}", self.next);
        self.docs.push(MockDoc {
            id: id.clone(),
            name: name.into(),
            path,
            editor: Editor::new(sprite),
            revision: 1,
            dirty: false,
            layer: 0,
            frame: 0,
            undo: Vec::new(),
            redo: Vec::new(),
        });
        self.active = Some(id.clone());
        id
    }

    fn find(&mut self, params: &Value) -> Result<usize, (i64, String)> {
        let id = params["id"].as_str().unwrap_or_default();
        self.docs.iter().position(|d| d.id == id).ok_or((-32004, format!("no document {id}")))
    }

    /// The "user" paints the whole first layer.
    pub fn user_fill(&mut self, i: usize, rgba: u32) {
        let d = &mut self.docs[i];
        let raw: *mut Editor = &mut d.editor;
        unsafe { pixmith_core::ffi::document::pixmith_fill(raw, 0, 0, 0, 0, rgba) };
        d.revision += 1;
        d.dirty = true;
    }
}

fn handle(state: &mut State, method: &str, p: &Value) -> Result<Value, (i64, String)> {
    state.calls.push(method.to_string());
    match method {
        "documents.list" => {
            let docs: Vec<Value> = state.docs.iter().map(|d| state.doc_json(d)).collect();
            Ok(json!({ "documents": docs, "active": state.active }))
        }
        "documents.get" => {
            let i = state.find(p)?;
            let d = &state.docs[i];
            let bytes = pxmt::write(&d.editor.sprite).map_err(|e| (-32000, e))?;
            Ok(
                json!({ "document": state.doc_json(d), "pxmt": base64::engine::general_purpose::STANDARD.encode(bytes) }),
            )
        }
        "documents.apply" => {
            if state.apply_fail_next > 0 {
                state.apply_fail_next -= 1;
                return Err((-32000, "could not apply the document".into()));
            }
            if state.busy_next > 0 {
                state.busy_next -= 1;
                return Err((-32010, "busy: a sheet is open".into()));
            }
            let i = state.find(p)?;
            if state.race_next > 0 {
                state.race_next -= 1;
                let c = state.race_color;
                state.user_fill(i, c);
            }
            let d = &mut state.docs[i];
            let base = p["base_revision"].as_u64().unwrap_or(u64::MAX);
            if base != d.revision {
                return Err((-32009, format!("stale revision {base}, current {}", d.revision)));
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(p["pxmt"].as_str().unwrap_or_default())
                .map_err(|e| (-32602, e.to_string()))?;
            let sprite = pxmt::read_document(&bytes).map_err(|e| (-32000, e))?;
            let label = p["label"].as_str().unwrap_or("edit").to_string();
            let old = std::mem::replace(&mut d.editor.sprite, sprite);
            d.undo.push((format!("AI: {label}"), old));
            d.redo.clear();
            d.revision += 1;
            d.dirty = true;
            let layers = d.editor.sprite.layers.len() as u64;
            let frames = d.editor.sprite.cels.len() as u64;
            if let Some(l) = p["layer"].as_u64() {
                d.layer = l.min(layers - 1) as u32;
            }
            if let Some(f) = p["frame"].as_u64() {
                d.frame = f.min(frames - 1) as u32;
            }
            let (id, layer, frame) = (d.id.clone(), p["layer"].as_u64(), p["frame"].as_u64());
            state.applies.push(ApplyRecord { id, label, base_revision: base, layer, frame });
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.undo" | "documents.redo" if state.undo_supported => {
            let i = state.find(p)?;
            let steps = p["steps"].as_u64().unwrap_or(1);
            let undo = method == "documents.undo";
            let d = &mut state.docs[i];
            let mut done = 0;
            for _ in 0..steps {
                let Some((label, sprite)) = (if undo { d.undo.pop() } else { d.redo.pop() }) else { break };
                let cur = std::mem::replace(&mut d.editor.sprite, sprite);
                if undo {
                    d.redo.push((label, cur))
                } else {
                    d.undo.push((label, cur))
                }
                d.revision += 1;
                done += 1;
            }
            Ok(json!({ "document": state.doc_json(&state.docs[i]), "steps": done }))
        }
        "documents.new" if p.get("pxmt").is_some() && !state.no_pxmt_new => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(p["pxmt"].as_str().unwrap_or_default())
                .map_err(|e| (-32602, e.to_string()))?;
            let sprite = pxmt::read_document(&bytes).map_err(|e| (-32000, e))?;
            let name = p["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Untitled {}", state.next + 1));
            let id = state.add(sprite, &name, None);
            let i = state.docs.iter().position(|d| d.id == id).unwrap();
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.new" => {
            let w = p["width"].as_u64().ok_or((-32602, "width".to_string()))? as u32;
            let h = p["height"].as_u64().ok_or((-32602, "height".to_string()))? as u32;
            let name = p["name"].as_str().map(str::to_string).unwrap_or_else(|| format!("Untitled {}", state.next + 1));
            let id = state.add(Sprite::new(w, h), &name, None);
            let i = state.docs.iter().position(|d| d.id == id).unwrap();
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.open" => {
            let path = p["path"].as_str().unwrap_or_default().to_string();
            if let Some(i) = state.docs.iter().position(|d| d.path.as_deref() == Some(path.as_str())) {
                state.active = Some(state.docs[i].id.clone());
                return Ok(json!({ "document": state.doc_json(&state.docs[i]) }));
            }
            let sprite = pxmt::open_file(&path).map_err(|e| (-32000, e))?;
            let name = Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let id = state.add(sprite, &name, Some(path));
            let i = state.docs.iter().position(|d| d.id == id).unwrap();
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.save" => {
            let i = state.find(p)?;
            let path = p["path"].as_str().map(str::to_string).or_else(|| state.docs[i].path.clone());
            let Some(path) = path else { return Err((-32000, "the document has no file yet".into())) };
            // Like the app: a file another tab has open cannot be saved to, unless `replace_open` closes a clean tab.
            if let Some(o) =
                state.docs.iter().position(|o| o.id != state.docs[i].id && o.path.as_deref() == Some(path.as_str()))
            {
                if p["replace_open"].as_bool().unwrap_or(false) && !state.docs[o].dirty {
                    let id = state.docs.remove(o).id;
                    state.active = state.docs.last().map(|d| d.id.clone());
                    let _ = id;
                } else {
                    return Err((
                        -32000,
                        format!("“{}” is already open in another tab. Close that tab first.", state.docs[o].name),
                    ));
                }
            }
            let i = state.docs.iter().position(|d| d.id == p["id"].as_str().unwrap_or_default()).unwrap();
            let d = &mut state.docs[i];
            pxmt::save_file(&path, &d.editor.sprite).map_err(|e| (-32000, e))?;
            d.path = Some(path);
            d.dirty = false;
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.activate" => {
            let i = state.find(p)?;
            state.active = Some(state.docs[i].id.clone());
            Ok(json!({ "document": state.doc_json(&state.docs[i]) }))
        }
        "documents.close" => {
            let i = state.find(p)?;
            if state.docs[i].dirty && !p["discard"].as_bool().unwrap_or(false) {
                return Err((-32011, "the document has unsaved changes".into()));
            }
            let id = state.docs.remove(i).id;
            if state.active.as_deref() == Some(id.as_str()) {
                state.active = state.docs.last().map(|d| d.id.clone());
            }
            Ok(json!({}))
        }
        "view.get" | "view.set" => {
            let id = p["id"].as_str().map(str::to_string).or_else(|| state.active.clone());
            let Some(id) = id else { return Ok(json!({ "document": null })) };
            let i = state.find(&json!({ "id": id }))?;
            if method == "view.set" {
                let d = &mut state.docs[i];
                if let Some(l) = p["layer"].as_u64() {
                    d.layer = (l as usize).min(d.editor.sprite.layers.len() - 1) as u32;
                }
                if let Some(f) = p["frame"].as_u64() {
                    d.frame = (f as usize).min(d.editor.sprite.cels.len() - 1) as u32;
                }
            }
            let d = &state.docs[i];
            Ok(json!({ "document": d.id, "layer": d.layer, "frame": d.frame, "zoom": 8, "tool": "pencil" }))
        }
        "game.capture" if !state.no_game_view => {
            if !state.game {
                return Err((-32012, "no running game".into()));
            }
            Ok(json!({ "png": base64::engine::general_purpose::STANDARD.encode(game_png()),
                       "w": GAME_W, "h": GAME_H, "client": "pixmith-godot" }))
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    }
}

fn serve(stream: TcpStream, state: Arc<Mutex<State>>, token: String, stop: Arc<AtomicBool>) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(100)));
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    let mut hello_done = false;
    state.lock().unwrap().clients += 1;
    let mut line = String::new();
    loop {
        if stop.load(Ordering::SeqCst) || state.lock().unwrap().drop_connections {
            break;
        }
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.ends_with('\n') => {}
            Ok(_) => continue,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => continue,
            Err(_) => break,
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => {
                let _ = writeln!(
                    writer,
                    "{}",
                    json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": "parse error" } })
                );
                break;
            }
        };
        line.clear();
        let (id, method, params) =
            (req["id"].clone(), req["method"].as_str().unwrap_or_default(), req["params"].clone());
        let result = if !hello_done {
            if method == "hello" && params["token"] == json!(token) && params["protocol"] == json!(1) {
                hello_done = true;
                state.lock().unwrap().hellos += 1;
                Ok(
                    json!({ "app": "Pixmith", "version": "9.9.9", "front_end": "mock", "platform": "macos", "protocol": 1 }),
                )
            } else {
                Err((-32001, "unauthorized".to_string()))
            }
        } else {
            handle(&mut state.lock().unwrap(), method, &params)
        };
        let unauthorized = matches!(&result, Err((-32001, _)));
        let resp = match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err((code, message)) => {
                json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
            }
        };
        if writeln!(writer, "{resp}").and_then(|_| writer.flush()).is_err() || unauthorized {
            break;
        }
    }
    state.lock().unwrap().clients -= 1;
}

pub struct MockApp {
    pub state: Arc<Mutex<State>>,
    pub port: u16,
    pub token: String,
    file: PathBuf,
    stop: Arc<AtomicBool>,
}

/// `<support>/Pixmith/Live` (`pixmith` on Linux), the folder the app writes its discovery file in.
pub fn live_dir(support: &Path) -> PathBuf {
    support.join(if cfg!(target_os = "linux") { "pixmith" } else { "Pixmith" }).join("Live")
}

impl MockApp {
    /// Starts a mock app that registers as this test process.
    pub fn start(support: &Path) -> MockApp {
        MockApp::start_as(support, std::process::id(), 1_729_760_000)
    }

    /// Starts a mock app whose discovery file claims `pid` and `started`.
    pub fn start_as(support: &Path, pid: u32, started: u64) -> MockApp {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let port = listener.local_addr().unwrap().port();
        let token = format!("{:032x}", (port as u128) * 7919 + started as u128);
        let state = Arc::new(Mutex::new(State { undo_supported: false, race_color: 0x0000ffff, ..Default::default() }));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (state, token, stop) = (state.clone(), token.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((s, _)) => {
                            let _ = s.set_nonblocking(false);
                            let (state, token, stop) = (state.clone(), token.clone(), stop.clone());
                            std::thread::spawn(move || serve(s, state, token, stop));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            });
        }
        let dir = live_dir(support);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(format!("{pid}.json"));
        let body = json!({ "protocol": 1, "pid": pid, "port": port, "token": token, "app": "Pixmith", "version": "9.9.9",
                           "front_end": "mock", "platform": "macos", "started": started });
        std::fs::write(&file, body.to_string()).unwrap();
        MockApp { state, port, token, file, stop }
    }

    /// The app quits: its port closes and its discovery file goes away.
    pub fn quit(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.file);
        // Give the accept loop and the connection threads a moment to drop their sockets.
        std::thread::sleep(Duration::from_millis(400));
    }

    /// A blank document the "user" created in the app; returns its id.
    pub fn add_blank(&self, w: u32, h: u32, name: &str) -> String {
        self.state.lock().unwrap().add(Sprite::new(w, h), name, None)
    }

    pub fn add_sprite(&self, sprite: Sprite, name: &str) -> String {
        self.state.lock().unwrap().add(sprite, name, None)
    }

    /// The composited color at (x, y) of the first frame of an app document.
    pub fn pixel(&self, id: &str, x: u32, y: u32) -> [u8; 4] {
        let s = self.state.lock().unwrap();
        let d = s.docs.iter().find(|d| d.id == id).expect("document");
        let px = d.editor.sprite.render(0);
        let i = ((y * d.editor.sprite.width + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    }

    pub fn revision(&self, id: &str) -> u64 {
        self.state.lock().unwrap().docs.iter().find(|d| d.id == id).expect("document").revision
    }

    pub fn doc_count(&self) -> usize {
        self.state.lock().unwrap().docs.len()
    }

    pub fn undo_labels(&self, id: &str) -> Vec<String> {
        let s = self.state.lock().unwrap();
        s.docs.iter().find(|d| d.id == id).expect("document").undo.iter().map(|(l, _)| l.clone()).collect()
    }

    pub fn view(&self, id: &str) -> (u32, u32) {
        let s = self.state.lock().unwrap();
        let d = s.docs.iter().find(|d| d.id == id).expect("document");
        (d.layer, d.frame)
    }

    pub fn calls(&self) -> Vec<String> {
        self.state.lock().unwrap().calls.clone()
    }

    /// How many `documents.apply` requests were made (also refused ones).
    pub fn applies(&self) -> usize {
        self.count("documents.apply")
    }

    pub fn count(&self, method: &str) -> usize {
        self.calls().iter().filter(|m| *m == method).count()
    }

    pub fn clear_calls(&self) {
        let mut s = self.state.lock().unwrap();
        s.calls.clear();
        s.applies.clear();
    }

    /// The "user" paints the whole first layer of an app document.
    pub fn user_fill(&self, id: &str, rgba: u32) {
        let mut s = self.state.lock().unwrap();
        let i = s.docs.iter().position(|d| d.id == id).expect("document");
        s.user_fill(i, rgba);
    }

    pub fn with<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        f(&mut self.state.lock().unwrap())
    }
}

impl Drop for MockApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::fs::remove_file(&self.file);
    }
}
