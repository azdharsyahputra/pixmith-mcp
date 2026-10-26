//! Live mode: when the Pixmith app is open on the same computer, the tools edit the documents open in the app
//! (the Live protocol of the Pixmith repository, docs/LIVE_PROTOCOL.md; README "Live mode"). Without an app,
//! everything stays headless.
//!
//! How an edit on an app document works: fetch the document as `.pxmt` bytes (`documents.get`), decode it into a
//! *shadow* document on the engine thread, run the unchanged tool on it, and send the result back with
//! `documents.apply` as one labelled undo step. A shadow is kept while the app's revision of the document is the
//! one the shadow was built from, so a selection, a floating paste or a free transform survives from one call to
//! the next; as soon as the user (or anything else) changes the document, the next call fetches it again.
//!
//! Calls never overlap: every tool call, including the ones that create, open and close documents in the app, runs
//! on the one engine thread (`engine.rs`), so the requests to the app are strictly one after another and parallel
//! tool calls from a client queue up there.
//!
//! Once this server has been connected to the app, a call that would create or edit a document does not silently
//! turn headless when the app is gone: it tries to reconnect once and otherwise fails (see [`Live::lost`]).

mod client;
mod discovery;

pub use client::AppDoc;
use client::{BUSY, Conn, INVALID_PARAMS, NO_GAME, NO_METHOD, RpcError, STALE, UNKNOWN_DOC, UNSAVED};

use crate::doc::Doc;
use crate::engine::{DocEntry, Engine, R};
use crate::paths::show;
use crate::tools::docs::{MAX_HEADLESS_DOCS, NewArgs, OpenArgs, OpenFormat, Target, summary};
use crate::tools::{Out, ToolDef, parse};
use base64::Engine as _;
use pixmith_core::Sprite;
use pixmith_core::formats::pxmt;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

/// How often an edit is redone on a fresh copy after the app reports a stale revision.
const STALE_RETRIES: usize = 3;
/// Waits before each retry of a request the app refused as busy.
const BUSY_DELAYS_MS: [u64; 6] = [40, 80, 160, 320, 640, 1000];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Use the app when one is running, otherwise edit headless.
    Auto,
    /// Require the app: tools fail with a clear message when none is running.
    On,
    /// Never talk to an app.
    Off,
}

impl Mode {
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" | "" => Ok(Mode::Auto),
            "on" | "1" | "true" | "yes" => Ok(Mode::On),
            "off" | "0" | "false" | "no" => Ok(Mode::Off),
            other => Err(format!("live mode {other:?} is not one of auto, on, off")),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Auto => "auto",
            Mode::On => "on",
            Mode::Off => "off",
        }
    }
}

/// A copy of an app document that tools run on.
struct Shadow {
    entry: DocEntry,
    meta: Meta,
}

/// What a shadow was built from.
struct Meta {
    /// The sprite as the app has it (the shadow's sprite is compared with it to see whether a tool changed anything).
    base: Sprite,
    revision: u64,
    /// The app's current layer and frame of the document when the shadow was last synchronized.
    app_layer: u32,
    app_frame: u32,
}

pub struct Live {
    pub mode: Mode,
    conn: Option<Conn>,
    /// `app_disconnect` was called: do not reconnect until `app_connect`.
    disabled: bool,
    /// Why the last connection attempt failed.
    problem: Option<String>,
    /// The server has been connected to an app at some point of this session.
    was_connected: bool,
    /// What broke the connection the last time it was lost (e.g. "Broken pipe").
    lost_cause: Option<String>,
    shadows: HashMap<String, Shadow>,
    /// Files the app's imported documents (PNG, GIF, sheet) came from, by app document id.
    sources: HashMap<String, std::path::PathBuf>,
}

impl Live {
    pub fn new(mode: Mode) -> Live {
        Live {
            mode,
            conn: None,
            disabled: false,
            problem: None,
            was_connected: false,
            lost_cause: None,
            shadows: HashMap::new(),
            sources: HashMap::new(),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }

    /// Looks for a running app (newest first, or the one with this pid) and connects to it.
    pub fn connect(&mut self, pid: Option<u32>) -> Result<(), String> {
        if self.mode == Mode::Off {
            return Err(self.not_connected());
        }
        let result = self.try_connect(pid);
        match &result {
            Ok(()) => self.problem = None,
            Err(e) => self.problem = Some(e.clone()),
        }
        result
    }

    fn try_connect(&mut self, pid: Option<u32>) -> Result<(), String> {
        let dir = discovery::live_dir()
            .ok_or("cannot locate the Pixmith app data folder (HOME or PIXMITH_SUPPORT_DIR is not set)")?;
        let entries: Vec<_> = discovery::scan(&dir, &discovery::pid_alive, true)
            .into_iter()
            .filter(|e| pid.is_none_or(|p| e.pid == p))
            .collect();
        if entries.is_empty() {
            return Err(match pid {
                Some(p) => format!("no running Pixmith app with pid {p} (looked in {})", dir.display()),
                None => format!("no running Pixmith app found (looked in {})", dir.display()),
            });
        }
        let mut why = Vec::new();
        for e in &entries {
            match Conn::connect(e) {
                Ok(c) => {
                    self.conn = Some(c);
                    self.was_connected = true;
                    self.lost_cause = None;
                    self.shadows.clear();
                    return Ok(());
                }
                Err(m) => why.push(m),
            }
        }
        Err(why.join("; "))
    }

    /// Connected, or connects now when the mode and `app_disconnect` allow it.
    pub fn ensure(&mut self) -> bool {
        if self.conn.is_some() {
            return true;
        }
        if self.mode == Mode::Off || self.disabled {
            return false;
        }
        self.connect(None).is_ok()
    }

    /// Allows (re)connecting again after `app_disconnect`.
    pub fn enable(&mut self) {
        self.disabled = false;
    }

    /// Stops lazy reconnecting until `app_connect`.
    pub fn set_disabled(&mut self) {
        self.disabled = true;
    }

    pub fn disconnect(&mut self) {
        self.conn = None;
        self.shadows.clear();
    }

    /// Closes a tab this server created in the app and could not finish building (best effort, reconnecting once).
    /// Returns whether the tab is gone.
    fn discard_tab(&mut self, id: &str) -> bool {
        if self.conn.is_none() && !self.ensure() {
            return false;
        }
        self.shadows.remove(id);
        self.sources.remove(id);
        self.rpc_busy("documents.close", json!({ "id": id, "discard": true })).is_ok()
    }

    /// The server was connected to the app earlier in this session.
    pub fn was_connected(&self) -> bool {
        self.was_connected
    }

    /// After [`ensure`](Self::ensure) failed: the error for a call that would create or edit a document, when the
    /// server had been connected to the app in this session (mode auto or on) and `app_disconnect` was not called.
    /// Such a call must not silently go headless. `None`: working headless is expected (never connected, live off,
    /// or disconnected on purpose).
    pub fn lost(&self) -> Option<String> {
        if !self.was_connected || self.mode == Mode::Off || self.disabled || self.conn.is_some() {
            return None;
        }
        let why = self.problem.as_deref().unwrap_or("no Pixmith app is running");
        let why = match &self.lost_cause {
            Some(c) => format!("{c}; {why}"),
            None => why.to_string(),
        };
        Some(format!(
            "lost connection to the Pixmith app: it was connected but is not reachable now ({why}); nothing was changed. Start it again and call app_connect (the next call also reconnects by itself), or pass target: \"headless\" (doc_new / doc_open) or a headless document id as `doc`, or call app_disconnect to work headless on purpose"
        ))
    }

    /// Why there is no connection, as a message for a tool error.
    pub fn not_connected(&self) -> String {
        match self.mode {
            Mode::Off => "live mode is off (--live off or PIXMITH_LIVE=off): this server edits headless only".into(),
            _ if self.disabled => {
                "disconnected from the Pixmith app by app_disconnect; call app_connect to use it again".into()
            }
            Mode::On => format!(
                "live mode is on (--live on) but no Pixmith app can be reached: {}. Start the Pixmith app and keep \"Allow AI agents to control Pixmith (Live mode)\" on in its Preferences, or restart the server with --live auto to fall back to headless editing",
                self.problem.as_deref().unwrap_or("no Pixmith app is running")
            ),
            Mode::Auto => format!(
                "not connected to the Pixmith app: {}",
                self.problem.as_deref().unwrap_or("no Pixmith app is running")
            ),
        }
    }

    // MARK: Requests

    fn rpc(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let Some(c) = self.conn.as_mut() else { return Err(RpcError::Io(self.not_connected())) };
        let r = c.request(method, params);
        if let Err(RpcError::Io(m)) = &r {
            self.conn = None;
            self.shadows.clear();
            self.problem = Some("the connection to the app was lost".into());
            self.lost_cause = Some(m.clone());
        }
        r
    }

    /// For requests that change nothing: after a lost connection, reconnects once and asks again.
    fn rpc_read(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        match self.rpc(method, params.clone()) {
            Err(RpcError::Io(_)) if self.ensure() => self.rpc(method, params),
            r => r,
        }
    }

    /// For requests that change something: waits and asks again while the app says it is busy.
    fn rpc_busy(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let mut delays = BUSY_DELAYS_MS.iter();
        loop {
            match self.rpc(method, params.clone()) {
                Err(RpcError::Rpc { code: BUSY, .. }) if let Some(ms) = delays.next() => {
                    std::thread::sleep(Duration::from_millis(*ms));
                }
                r => return r,
            }
        }
    }

    /// A screenshot of the running Godot game (`game.capture`): the PNG bytes, its size and the game's client name.
    pub fn game_capture(&mut self, timeout_ms: u64) -> R<(Vec<u8>, u32, u32, String)> {
        if !self.ensure() {
            return Err(format!(
                "game_screenshot needs the running Pixmith app (live mode) and a game with the Pixmith Live Link addon: {}",
                self.not_connected()
            ));
        }
        let v = self.rpc_read("game.capture", json!({ "timeout_ms": timeout_ms })).map_err(|e| match e {
            RpcError::Rpc { code: NO_GAME, .. } => "no running game with the Pixmith Live Link addon answered; start the game from Godot (debug) with the addon enabled".to_string(),
            RpcError::Rpc { code: NO_METHOD, .. } => "this Pixmith app does not support the game view; update Pixmith to 0.5.1 or later".to_string(),
            other => explain(&other, "the app"),
        })?;
        let png = v
            .get("png")
            .and_then(Value::as_str)
            .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
            .ok_or("the Pixmith app answered game.capture without a valid PNG")?;
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        let client = v.get("client").and_then(Value::as_str).unwrap_or("").to_string();
        Ok((png, n("w"), n("h"), client))
    }

    /// The app's open documents and the active one.
    pub fn list(&mut self) -> Result<(Vec<AppDoc>, Option<String>), RpcError> {
        let v = self.rpc_read("documents.list", json!({}))?;
        let docs = v
            .get("documents")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(AppDoc::parse).collect::<Result<Vec<_>, _>>())
            .transpose()?
            .unwrap_or_default();
        let active = v
            .get("active")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| docs.iter().find(|d| d.active).map(|d| d.id.clone()));
        self.shadows.retain(|id, _| docs.iter().any(|d| d.id == *id));
        self.sources.retain(|id, _| docs.iter().any(|d| d.id == *id));
        Ok((docs, active))
    }

    fn get(&mut self, id: &str) -> Result<(AppDoc, Vec<u8>), RpcError> {
        let v = self.rpc_read("documents.get", json!({ "id": id }))?;
        let doc = AppDoc::parse(v.get("document").unwrap_or(&Value::Null))?;
        let b64 = v.get("pxmt").and_then(Value::as_str).ok_or_else(|| RpcError::Io("the app sent no pxmt".into()))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| RpcError::Io(format!("the app sent invalid base64: {e}")))?;
        Ok((doc, bytes))
    }

    #[allow(clippy::too_many_arguments)]
    fn apply(
        &mut self,
        id: &str,
        base_revision: u64,
        sprite: &Sprite,
        label: &str,
        layer: Option<u32>,
        frame: Option<u32>,
    ) -> Result<AppDoc, RpcError> {
        let bytes = pxmt::write(sprite).map_err(|e| RpcError::Io(format!("cannot encode the document: {e}")))?;
        let mut params = json!({
            "id": id, "base_revision": base_revision, "label": label,
            "pxmt": base64::engine::general_purpose::STANDARD.encode(bytes),
        });
        if let Some(l) = layer {
            params["layer"] = json!(l);
        }
        if let Some(f) = frame {
            params["frame"] = json!(f);
        }
        let v = self.rpc_busy("documents.apply", params)?;
        AppDoc::parse(v.get("document").unwrap_or(&Value::Null))
    }

    /// A shadow of `doc`: the one kept from the last call when the app's revision is still the same, otherwise a
    /// fresh copy fetched from the app.
    fn checkout(&mut self, doc: &AppDoc, force_fetch: bool) -> R<Shadow> {
        if !force_fetch
            && let Some(mut sh) = self.shadows.remove(&doc.id)
            && sh.meta.revision == doc.revision
        {
            sh.sync_with(doc);
            return Ok(sh);
        }
        self.shadows.remove(&doc.id);
        let (fresh, bytes) = self.get(&doc.id).map_err(|e| explain(&e, &doc.tool_id()))?;
        let sprite = pxmt::read_document(&bytes)
            .map_err(|e| format!("cannot read {} as the app sent it: {e}", doc.tool_id()))?;
        let base = sprite.clone();
        let entry = DocEntry {
            id: fresh.tool_id(),
            name: fresh.name.clone(),
            doc: Doc::from_sprite(sprite),
            path: None,
            source: None,
            layer: 0,
            frame: 0,
            txns: Vec::new(),
            asked: Default::default(),
        };
        let mut sh = Shadow { entry, meta: Meta { base, revision: fresh.revision, app_layer: 0, app_frame: 0 } };
        sh.entry.doc.mark_saved();
        sh.sync_with(&fresh);
        Ok(sh)
    }

    /// `(tool id, name)` of the app's documents, empty when there is no app.
    pub fn app_documents(&mut self) -> Vec<(String, String)> {
        if !self.ensure() {
            return Vec::new();
        }
        self.list().map(|(d, _)| d.iter().map(|d| (d.tool_id(), d.name.clone())).collect()).unwrap_or_default()
    }

    /// The JSON of `app_status` (the headless document ids are passed in by the caller, who owns them).
    pub fn status(&mut self, headless: Vec<String>) -> Value {
        let mut v =
            json!({ "mode": self.mode.name(), "connected": self.is_connected(), "headless_documents": headless });
        if self.mode != Mode::Off && !self.disabled && self.conn.is_none() {
            self.ensure();
        }
        match self.conn.as_ref().map(|c| c.info.clone()) {
            Some(info) => {
                v["connected"] = json!(true);
                v["app"] = json!({
                    "name": info.app, "version": info.version, "front_end": info.front_end,
                    "platform": info.platform, "pid": info.pid, "port": info.port, "protocol": discovery::PROTOCOL,
                });
                match self.list() {
                    Ok((docs, active)) => {
                        v["documents"] = Value::Array(docs.iter().map(AppDoc::json).collect());
                        v["active"] = json!(docs.iter().find(|d| Some(&d.id) == active.as_ref()).map(AppDoc::tool_id));
                    }
                    Err(e) => v["documents_error"] = json!(e.to_string()),
                }
                v["note"] = json!(
                    "Tools edit the app's documents (ids app:<id>; `doc` may be omitted for the active one). Each call is an undo step named \"AI: <tool>\" in the app."
                );
            }
            None => {
                v["connected"] = json!(false);
                let lost = self.lost();
                v["reason"] = json!(lost.clone().unwrap_or_else(|| self.not_connected()));
                v["note"] = json!(match self.mode {
                    _ if lost.is_some() =>
                        "The app was connected earlier in this session but is not reachable now: calls that create or edit documents fail instead of silently going headless. Start the app again (the next call reconnects), or call app_disconnect / pass target: \"headless\" to work headless on purpose.",
                    Mode::Off => "Live mode is off; tools edit headless documents (ids d1, d2, ...).",
                    _ =>
                        "Tools edit headless documents (ids d1, d2, ...). Open the Pixmith app and call app_connect to edit its documents live.",
                });
            }
        }
        v
    }
}

impl Shadow {
    /// Takes the app's name, path, dirty flag and current layer and frame.
    fn sync_with(&mut self, doc: &AppDoc) {
        let e = &mut self.entry;
        e.name = doc.name.clone();
        e.path = doc.path.as_deref().map(Into::into);
        if doc.dirty && !e.doc.is_dirty() {
            e.doc.touch();
        } else if !doc.dirty && e.doc.is_dirty() {
            e.doc.mark_saved();
        }
        e.layer = doc.layer;
        e.frame = doc.frame;
        e.clamp_active();
        e.asked.set((None, None));
        self.meta.app_layer = doc.layer;
        self.meta.app_frame = doc.frame;
    }
}

/// An error from the app as a tool error message.
fn explain(err: &RpcError, doc: &str) -> String {
    match err {
        RpcError::Rpc { code: UNKNOWN_DOC, .. } => {
            format!("{doc} is not open in the Pixmith app (any more); see doc_list")
        }
        RpcError::Rpc { code: BUSY, .. } => "the Pixmith app is busy (a dialog is open or a stroke is in progress); nothing was changed, try again in a moment".into(),
        RpcError::Rpc { message, .. } => message.clone(),
        RpcError::Io(m) => format!("lost connection to the Pixmith app ({m}); reconnect with app_connect once the app is running again"),
    }
}

// MARK: Dispatch

/// A tool call as the server sees it.
pub struct Call {
    pub def: ToolDef,
    /// The tool has a `doc` argument.
    pub takes_doc: bool,
}

/// Runs a tool: on the headless documents, or on the app's when the call targets one.
pub fn dispatch(e: &mut Engine, call: &Call, args: Value) -> R<Out> {
    let name = call.def.name;
    // Closing all headless documents never involves the app.
    if name == "doc_close" && args.get("all_headless").and_then(Value::as_bool) == Some(true) {
        return (call.def.run)(e, args);
    }
    let mut out = dispatch_routed(e, call, args)?;
    if matches!(name, "doc_new" | "doc_open") {
        limit_headless(e, &mut out);
    }
    Ok(out)
}

/// Keeps the number of headless documents bounded: past [`MAX_HEADLESS_DOCS`] the oldest unmodified ones are closed
/// (never the one just created) and the result says so.
fn limit_headless(e: &mut Engine, out: &mut Out) {
    let Some(newest) = out.value.get("doc").and_then(Value::as_str).filter(|d| !d.starts_with("app:")) else { return };
    let newest = newest.to_string();
    let closed = e.trim_headless(MAX_HEADLESS_DOCS, &newest);
    let left = e.headless_count();
    let note = if !closed.is_empty() {
        format!(
            "more than {MAX_HEADLESS_DOCS} headless documents were open, so the oldest unmodified ones were closed: {}. Close documents you are done with (doc_close; all_headless=true closes them all)",
            closed.join(", ")
        )
    } else if left > MAX_HEADLESS_DOCS {
        format!(
            "{left} headless documents are open (limit {MAX_HEADLESS_DOCS}) and the rest have unsaved changes; save or close them (doc_close; all_headless=true with discard_changes=true closes them all)"
        )
    } else {
        return;
    };
    if let Some(o) = out.value.as_object_mut() {
        o.insert("note".into(), json!(note));
        if !closed.is_empty() {
            o.insert("closed_headless".into(), json!(closed));
        }
    }
}

fn dispatch_routed(e: &mut Engine, call: &Call, args: Value) -> R<Out> {
    let name = call.def.name;
    let creates = matches!(name, "doc_new" | "doc_open");
    if !(call.takes_doc || creates || name == "doc_list") {
        return (call.def.run)(e, args);
    }
    if e.live.mode == Mode::Off {
        let app_ref = args.get("doc").and_then(Value::as_str).is_some_and(|d| client::strip_prefix(d).is_some());
        if app_ref || args.get("target").and_then(Value::as_str) == Some("app") {
            return Err(e.live.not_connected());
        }
        return (call.def.run)(e, args);
    }
    match name {
        "doc_list" => return list_documents(e, call, args),
        "doc_new" | "doc_open" => return create_document(e, call, args),
        _ => {}
    }
    let implicit = args.get("doc").and_then(Value::as_str).is_none_or(|d| d.trim().is_empty());
    match route(e, &args)? {
        None => {
            let mut out = (call.def.run)(e, args)?;
            // Connected, but the app has no document to take this call: say that it went to a headless one.
            if implicit
                && e.live.is_connected()
                && let Some(o) = out.value.as_object_mut()
            {
                o.entry("target").or_insert(json!("headless"));
            }
            Ok(out)
        }
        Some(doc) => on_app(e, call, args, doc),
    }
}

/// The app document a call is aimed at, or `None` for the headless engine.
fn route(e: &mut Engine, args: &Value) -> R<Option<AppDoc>> {
    let given = args.get("doc").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
    if let Some(d) = given {
        if let Some(id) = client::strip_prefix(d) {
            if !e.live.ensure() {
                return Err(e.live.not_connected());
            }
            let (docs, _) = e.live.list().map_err(|x| explain(&x, d))?;
            return docs
                .iter()
                .find(|x| x.id == id)
                .cloned()
                .map(Some)
                .ok_or_else(|| format!("the Pixmith app has no document {d:?} (open in the app: {})", ids(&docs)));
        }
        if e.docs.iter().any(|x| x.id == d || x.name == d) {
            return Ok(None);
        }
        // A bare id as the app names it ("doc-3").
        if e.live.ensure()
            && let Ok((docs, _)) = e.live.list()
            && let Some(x) = docs.iter().find(|x| x.id == d)
        {
            return Ok(Some(x.clone()));
        }
        return Ok(None);
    }
    if !e.live.ensure() {
        return if e.live.mode == Mode::On {
            Err(e.live.not_connected())
        } else if let Some(lost) = e.live.lost() {
            Err(lost)
        } else {
            Ok(None)
        };
    }
    let (docs, active) = match e.live.list() {
        Ok(x) => x,
        Err(x) if e.live.mode == Mode::On => return Err(explain(&x, "the app")),
        Err(x) if e.live.was_connected() => return Err(e.live.lost().unwrap_or_else(|| explain(&x, "the app"))),
        Err(_) => return Ok(None),
    };
    let pick = active.as_ref().and_then(|a| docs.iter().find(|d| &d.id == a)).or(match docs.as_slice() {
        [only] => Some(only),
        _ => None,
    });
    if let Some(d) = pick {
        return Ok(Some(d.clone()));
    }
    if docs.is_empty() {
        if !e.docs.is_empty() {
            return Ok(None);
        }
        return Err(
            "no document is open in the Pixmith app; call doc_new or doc_open (they create it in the app)".into()
        );
    }
    Err(format!("several documents are open in the Pixmith app and none is active; pass `doc` ({})", ids(&docs)))
}

fn ids(docs: &[AppDoc]) -> String {
    if docs.is_empty() {
        return "none".into();
    }
    docs.iter().map(|d| format!("{} ({})", d.tool_id(), d.name)).collect::<Vec<_>>().join(", ")
}

fn list_documents(e: &mut Engine, call: &Call, args: Value) -> R<Out> {
    let mut out = (call.def.run)(e, args)?;
    let connected = e.live.ensure();
    let Some(obj) = out.value.as_object_mut() else { return Ok(out) };
    if !connected {
        obj.insert(
            "app".into(),
            json!({ "connected": false, "mode": e.live.mode.name(),
                    "reason": e.live.lost().unwrap_or_else(|| e.live.not_connected()) }),
        );
        return Ok(out);
    }
    let (docs, active) = e.live.list().map_err(|x| explain(&x, "the app"))?;
    let mut all: Vec<Value> = docs.iter().map(AppDoc::json).collect();
    if let Some(Value::Array(h)) = obj.remove("documents") {
        all.extend(h.into_iter().map(|mut d| {
            d["target"] = json!("headless");
            d
        }));
    }
    obj.insert("documents".into(), Value::Array(all));
    obj.insert("active".into(), json!(docs.iter().find(|d| Some(&d.id) == active.as_ref()).map(AppDoc::tool_id)));
    let info = e.live.conn.as_ref().map(|c| c.info.clone());
    obj.insert(
        "app".into(),
        json!({ "connected": true, "mode": e.live.mode.name(), "name": info.as_ref().map(|i| i.app.clone()),
                "version": info.as_ref().map(|i| i.version.clone()), "front_end": info.map(|i| i.front_end) }),
    );
    Ok(out)
}

// MARK: New and open documents

fn create_document(e: &mut Engine, call: &Call, mut args: Value) -> R<Out> {
    let target = match args.get("target").and_then(Value::as_str) {
        None => None,
        Some("app") => Some(Target::App),
        Some("headless") => Some(Target::Headless),
        Some(other) => return Err(format!("target must be \"app\" or \"headless\", not {other:?}")),
    };
    let mut connected = target != Some(Target::Headless) && e.live.ensure();
    if connected && let Err(x) = e.live.list() {
        // A connection that broke since the last call: `list` already reconnected once.
        connected = false;
        if target.is_none() && e.live.mode != Mode::On {
            return Err(e.live.lost().unwrap_or_else(|| explain(&x, "the app")));
        }
    }
    if target == Some(Target::App) && !connected {
        return Err(e.live.not_connected());
    }
    if target.is_none() && !connected {
        if e.live.mode == Mode::On {
            return Err(e.live.not_connected());
        }
        if let Some(lost) = e.live.lost() {
            return Err(lost);
        }
    }
    if !connected {
        // Headless on purpose (or the app was never there): when an app was involved in this session, say so.
        let mut out = (call.def.run)(e, args)?;
        if (target == Some(Target::Headless) || e.live.was_connected())
            && let Some(o) = out.value.as_object_mut()
        {
            o.insert("target".into(), json!("headless"));
        }
        return Ok(out);
    }
    if let Some(o) = args.as_object_mut() {
        o.remove("target");
    }
    if call.def.name == "doc_new" { new_in_app(e, call, args) } else { open_in_app(e, call, args) }
}

/// A one-line summary of an app document, built on its shadow.
fn app_summary(e: &mut Engine, doc: &AppDoc) -> R<Value> {
    let sh = e.live.checkout(doc, false)?;
    let mut v = summary(&sh.entry);
    v["target"] = json!("app");
    v["revision"] = json!(doc.revision);
    e.live.shadows.insert(doc.id.clone(), sh);
    Ok(v)
}

fn created_doc(v: &Value) -> R<AppDoc> {
    AppDoc::parse(v.get("document").unwrap_or(&Value::Null)).map_err(|x| explain(&x, "the new document"))
}

fn new_in_app(e: &mut Engine, call: &Call, args: Value) -> R<Out> {
    let a: NewArgs = parse(args.clone())?;
    // The regular tool builds the document (background, color mode, palette preset) and validates the arguments.
    (call.def.run)(e, args)?;
    let temp = e.take_last().ok_or("internal error: no document was created")?;
    let extras = a.palette_preset.is_some()
        || a.background.is_some_and(|c| c.0 & 0xFF != 0)
        || a.color_mode.is_some_and(|m| m.code() != 0);
    let doc = new_tab(e, a.name.as_deref(), temp.doc.sprite(), "doc_new", extras, "the new document")?;
    match app_summary(e, &doc) {
        Ok(s) => Out::json(s),
        Err(m) => Err(abandon(e, &doc, m)),
    }
}

fn open_in_app(e: &mut Engine, call: &Call, args: Value) -> R<Out> {
    let a: OpenArgs = parse(args.clone())?;
    let path = e.sandbox.read(&a.path)?;
    if path.is_dir() {
        return Err(format!("{}: is a directory", a.path));
    }
    let ext = path.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if crate::tools::docs::open_format(&a, &ext) == OpenFormat::Native {
        let v = e.live.rpc_busy("documents.open", json!({ "path": show(&path) })).map_err(|x| explain(&x, &a.path))?;
        return Out::json(app_summary(e, &created_doc(&v)?)?);
    }
    // PNG, GIF and sheet imports are built here and handed to a new tab.
    (call.def.run)(e, args)?;
    let temp = e.take_last().ok_or("internal error: no document was created")?;
    let doc = new_tab(e, Some(&temp.name), temp.doc.sprite(), "doc_open", true, &a.path)?;
    let mut s = match app_summary(e, &doc) {
        Ok(s) => s,
        Err(m) => return Err(abandon(e, &doc, m)),
    };
    s["imported_from"] = json!(show(&path));
    e.live.sources.insert(doc.id.clone(), path);
    Out::json(s)
}

/// Creates a tab in the app holding `sprite`. Newer apps take the whole document in one `documents.new` request
/// (`pxmt`), so the tab appears complete or not at all. An app that does not know the parameter answers -32602;
/// then the tab is made at the right size and the sprite applied to it, and a failure of that second step closes the
/// tab again (`apply_after`: false skips the apply for a plain blank document).
fn new_tab(
    e: &mut Engine,
    name: Option<&str>,
    sprite: &Sprite,
    label: &str,
    apply_after: bool,
    what: &str,
) -> R<AppDoc> {
    let bytes = pxmt::write(sprite).map_err(|x| format!("cannot encode the document: {x}"))?;
    let mut params = json!({ "pxmt": base64::engine::general_purpose::STANDARD.encode(bytes) });
    if let Some(n) = name {
        params["name"] = json!(n);
    }
    match e.live.rpc_busy("documents.new", params) {
        Ok(v) => return created_doc(&v),
        Err(RpcError::Rpc { code: INVALID_PARAMS, .. }) => {}
        Err(x) => return Err(explain(&x, what)),
    }
    // An older app: make the tab, then put the document into it.
    let mut params = json!({ "width": sprite.width, "height": sprite.height });
    if let Some(n) = name {
        params["name"] = json!(n);
    }
    let v = e.live.rpc_busy("documents.new", params).map_err(|x| explain(&x, what))?;
    let created = created_doc(&v)?;
    if !apply_after {
        return Ok(created);
    }
    match e.live.apply(&created.id, created.revision, sprite, label, None, None) {
        Ok(d) => Ok(d),
        Err(x) => Err(abandon(e, &created, explain(&x, &created.tool_id()))),
    }
}

/// A call that created a tab in the app failed before the document was complete: closes the half-built tab so the
/// user is not left with an empty or one-frame sprite, and says so in the error.
fn abandon(e: &mut Engine, doc: &AppDoc, message: String) -> String {
    if e.live.discard_tab(&doc.id) {
        format!(
            "{message}. The tab {} that had been created for it was closed again; nothing was left in the app",
            doc.tool_id()
        )
    } else {
        format!(
            "{message}. The tab {} that had been created for it could not be closed; close it by hand (doc_close with discard_changes=true, or in the app)",
            doc.tool_id()
        )
    }
}

// MARK: Tools on an app document

fn on_app(e: &mut Engine, call: &Call, mut args: Value, doc: AppDoc) -> R<Out> {
    let name = call.def.name;
    if let Some(o) = args.as_object_mut() {
        o.insert("doc".into(), json!(doc.tool_id()));
    }
    match name {
        "doc_save" => save_in_app(e, args, doc),
        "doc_close" => close_in_app(e, args, doc),
        "undo" | "redo" => step_in_app(e, args, doc, name),
        "history" | "history_goto" => Err(format!(
            "{name} is not available in live mode: the Pixmith app keeps the undo history of {}. Every edit made through this server is a step named \"AI: <tool>\" there; use Cmd/Ctrl+Z (Edit menu) in the app",
            doc.tool_id()
        )),
        "transaction" => Out::json(json!({
            "doc": doc.tool_id(),
            "open_transactions": [],
            "note": "transactions are not used in live mode: every tool call is its own undo step named \"AI: <tool>\" in the app",
        })),
        _ => run_on_shadow(e, call, args, doc),
    }
}

/// A label for the undo step: the tool and its action or shape.
fn label_for(name: &str, args: &Value) -> String {
    let sub = ["action", "shape"].iter().find_map(|k| args.get(*k).and_then(Value::as_str));
    match sub {
        Some(s) => format!("{name} {s}"),
        None => name.to_string(),
    }
}

fn run_on_shadow(e: &mut Engine, call: &Call, args: Value, mut doc: AppDoc) -> R<Out> {
    let name = call.def.name;
    let tool_id = doc.tool_id();
    let label = label_for(name, &args);
    for attempt in 0..=STALE_RETRIES {
        let sh = e.live.checkout(&doc, attempt > 0)?;
        let Shadow { entry, mut meta } = sh;
        e.docs.retain(|d| d.id != tool_id);
        e.docs.push(entry);
        let result = (call.def.run)(e, args.clone());
        let Some(at) = e.docs.iter().position(|d| d.id == tool_id) else {
            return Err("internal error: the document disappeared during the call".into());
        };
        let entry = e.docs.remove(at);
        let changed = *entry.doc.sprite() != meta.base;
        let mut out = match result {
            Ok(o) => o,
            Err(msg) => {
                // A failed call changes nothing; keep the copy (and its selection) unless it did change.
                if !changed {
                    e.live.shadows.insert(doc.id.clone(), Shadow { entry, meta });
                }
                return Err(msg);
            }
        };
        if name == "doc_info"
            && let Some(h) = out.value.get_mut("history")
        {
            *h = json!({ "managed_by": "the Pixmith app", "note": "the undo history lives in the app (Cmd/Ctrl+Z)" });
        }
        if call.def.hints.read_only {
            e.live.shadows.insert(doc.id.clone(), Shadow { entry, meta });
            return Ok(out);
        }
        // Where the app's current layer and frame should go: wherever the tool made them go, or where it was aimed.
        let top = entry.layer_count().saturating_sub(1);
        let last_frame = entry.doc.frame_count().saturating_sub(1);
        let (asked_layer, asked_frame) = entry.asked.get();
        let layer =
            if entry.layer != meta.app_layer { entry.layer } else { asked_layer.unwrap_or(meta.app_layer) }.min(top);
        let frame = if entry.frame != meta.app_frame { entry.frame } else { asked_frame.unwrap_or(meta.app_frame) }
            .min(last_frame);
        let new_layer = (layer != meta.app_layer).then_some(layer);
        let new_frame = (frame != meta.app_frame).then_some(frame);
        if !changed {
            let mut view = serde_json::Map::new();
            view.insert("id".into(), json!(doc.id));
            if let Some(l) = new_layer {
                view.insert("layer".into(), json!(l));
            }
            if let Some(f) = new_frame {
                view.insert("frame".into(), json!(f));
            }
            let switch = name == "doc_set_active"
                && args.get("layer").is_none_or(Value::is_null)
                && args.get("frame").is_none_or(Value::is_null);
            if switch {
                e.live.rpc_busy("documents.activate", json!({ "id": doc.id })).map_err(|x| explain(&x, &tool_id))?;
            } else if view.len() > 1 {
                e.live.rpc_busy("view.set", Value::Object(view)).map_err(|x| explain(&x, &tool_id))?;
                meta.app_layer = layer;
                meta.app_frame = frame;
            }
            e.live.shadows.insert(doc.id.clone(), Shadow { entry, meta });
            return Ok(out);
        }
        match e.live.apply(&doc.id, meta.revision, entry.doc.sprite(), &label, new_layer, new_frame) {
            Ok(applied) => {
                meta.base = entry.doc.sprite().clone();
                meta.revision = applied.revision;
                meta.app_layer = applied.layer;
                meta.app_frame = applied.frame;
                e.live.shadows.insert(doc.id.clone(), Shadow { entry, meta });
                return Ok(out);
            }
            Err(RpcError::Rpc { code: STALE, .. }) => {
                // The document changed in the app since it was fetched: fetch again and redo the edit there.
                let (docs, _) = e.live.list().map_err(|x| explain(&x, &tool_id))?;
                doc = docs
                    .into_iter()
                    .find(|d| d.id == doc.id)
                    .ok_or_else(|| format!("{tool_id} is not open in the Pixmith app (any more); see doc_list"))?;
            }
            Err(RpcError::Io(m)) => {
                return Err(format!(
                    "lost connection to the Pixmith app ({m}); the edit may or may not have reached the app, check with doc_info after reconnecting with app_connect. Nothing was applied to a headless copy"
                ));
            }
            Err(other) => return Err(explain(&other, &tool_id)),
        }
    }
    Err(format!(
        "{tool_id} kept changing in the Pixmith app (stale revision {} times in a row); nothing was changed, try again",
        STALE_RETRIES + 1
    ))
}

fn save_in_app(e: &mut Engine, args: Value, doc: AppDoc) -> R<Out> {
    let tool_id = doc.tool_id();
    let path = args.get("path").and_then(Value::as_str).map(str::to_string);
    let copy = args.get("copy").and_then(Value::as_bool).unwrap_or(false);
    let replace = args.get("replace").and_then(Value::as_bool).unwrap_or(false);
    let mut defaulted = false;
    let target = match (&path, &doc.path) {
        (Some(p), _) => e.sandbox.write(p)?,
        (None, Some(p)) => e.sandbox.write(p)?,
        (None, None) => match e.live.sources.get(&doc.id) {
            Some(src) => {
                defaulted = true;
                e.sandbox.write(&crate::tools::docs::default_save_path(src).to_string_lossy())?
            }
            None => {
                return Err(format!(
                    "{tool_id} has no file yet; pass path (e.g. \"{}.pxmt\" or \"{}.aseprite\")",
                    doc.name, doc.name
                ));
            }
        },
    };
    let ext = target.extension().map(|x| x.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "pxmt" | "aseprite" | "ase") {
        return Err(format!(
            "{}: save writes .pxmt, .aseprite or .ase; use the export tool for .{ext} files",
            show(&target)
        ));
    }
    if copy {
        if path.is_none() {
            return Err("copy=true needs a path".into());
        }
        let sh = e.live.checkout(&doc, false)?;
        let saved = pxmt::save_file(&target.to_string_lossy(), sh.entry.doc.sprite());
        e.live.shadows.insert(doc.id.clone(), sh);
        saved.map_err(|x| format!("could not save {}: {x}", show(&target)))?;
        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        return Out::json(
            json!({ "doc": tool_id, "saved": show(&target), "format": ext, "bytes": size, "copy": true, "dirty": doc.dirty }),
        );
    }
    // The app refuses to save to a file that another of its tabs has open.
    let mut closed_other = Vec::new();
    let (all, _) = e.live.list().map_err(|x| explain(&x, &tool_id))?;
    for other in all.iter().filter(|o| o.id != doc.id && o.path.as_deref().is_some_and(|p| same_file(p, &target))) {
        let who = format!("{} ({})", other.tool_id(), other.name);
        if !replace {
            return Err(format!(
                "{} is already open in the app as {who}, and the app cannot save two tabs to one file. Close that tab first (doc_close), or pass replace: true to close it automatically when it has no unsaved changes",
                show(&target)
            ));
        }
        if other.dirty {
            return Err(format!(
                "{} is open in the app as {who} with unsaved changes, so replace did not close it and nothing was saved. Save it elsewhere (doc_save) or discard it (doc_close with discard_changes=true), then retry",
                show(&target)
            ));
        }
        match e.live.rpc_busy("documents.close", json!({ "id": other.id, "discard": false })) {
            Ok(_) => {
                e.live.shadows.remove(&other.id);
                e.live.sources.remove(&other.id);
                closed_other.push(other.tool_id());
            }
            Err(RpcError::Rpc { code: UNSAVED, .. }) => {
                return Err(format!("{who} has unsaved changes, so replace did not close it and nothing was saved"));
            }
            Err(x) => return Err(explain(&x, &other.tool_id())),
        }
    }
    let mut params = json!({ "id": doc.id, "path": show(&target) });
    if replace {
        // Newer apps close a clean tab holding the file themselves; older ones ignore the field.
        params["replace_open"] = json!(true);
    }
    let v = e.live.rpc_busy("documents.save", params).map_err(|x| {
        let m = explain(&x, &tool_id);
        if m.contains("already open in another tab") {
            format!("{m}. Pass replace: true to close that tab automatically when it has no unsaved changes")
        } else {
            m
        }
    })?;
    let saved = AppDoc::parse(v.get("document").unwrap_or(&Value::Null)).map_err(|x| explain(&x, &tool_id))?;
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    let mut out = json!({ "doc": tool_id, "saved": show(&target), "format": ext, "bytes": size, "copy": false, "dirty": saved.dirty });
    if defaulted {
        out["note"] = json!(format!("no path given: saved next to the imported file as {}", show(&target)));
    }
    if !closed_other.is_empty() {
        out["closed_tabs"] = json!(closed_other);
    }
    Out::json(out)
}

/// Whether an app document path and a target are the same file.
fn same_file(app_path: &str, target: &std::path::Path) -> bool {
    let a = std::path::Path::new(app_path);
    a == target || matches!((a.canonicalize(), target.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

fn close_in_app(e: &mut Engine, args: Value, doc: AppDoc) -> R<Out> {
    let tool_id = doc.tool_id();
    let discard = args.get("discard_changes").and_then(Value::as_bool).unwrap_or(false);
    match e.live.rpc_busy("documents.close", json!({ "id": doc.id, "discard": discard })) {
        Ok(_) => {}
        Err(RpcError::Rpc { code: UNSAVED, .. }) => {
            return Err(format!(
                "{tool_id} ({}) has unsaved changes; save it with doc_save or pass discard_changes=true",
                doc.name
            ));
        }
        Err(x) => return Err(explain(&x, &tool_id)),
    }
    e.live.shadows.remove(&doc.id);
    e.live.sources.remove(&doc.id);
    let open = e.live.list().map(|(d, _)| d.iter().map(AppDoc::tool_id).collect::<Vec<_>>()).unwrap_or_default();
    Out::json(json!({ "closed": tool_id, "open": open }))
}

/// Undo and redo go to the app's own history (`documents.undo` / `documents.redo`, a proposed extension of the
/// protocol); an app without them answers "method not found" and the message says how to undo by hand.
fn step_in_app(e: &mut Engine, args: Value, doc: AppDoc, name: &str) -> R<Out> {
    let tool_id = doc.tool_id();
    let steps = args.get("steps").and_then(Value::as_u64).unwrap_or(1).clamp(1, 10000);
    match e.live.rpc_busy(&format!("documents.{name}"), json!({ "id": doc.id, "steps": steps })) {
        Ok(v) => {
            let after = AppDoc::parse(v.get("document").unwrap_or(&Value::Null)).map_err(|x| explain(&x, &tool_id))?;
            let done = v.get("steps").and_then(Value::as_u64).unwrap_or(0);
            e.live.shadows.remove(&doc.id);
            let mut out = json!({ "doc": tool_id, "dirty": after.dirty });
            out[if name == "undo" { "undone" } else { "redone" }] = json!(done);
            if done == 0 {
                out["note"] = json!(if name == "undo" { "nothing to undo" } else { "nothing to redo" });
            }
            Out::json(out)
        }
        Err(RpcError::Rpc { code: NO_METHOD, .. }) => Err(format!(
            "{name} is not supported in live mode by this version of the Pixmith app; use Cmd/Ctrl+Z (Edit menu) in the app. Every edit made through this server is a step named \"AI: <tool>\" there"
        )),
        Err(x) => Err(explain(&x, &tool_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_parse_from_flags_and_environment_values() {
        for (s, m) in [
            ("auto", Mode::Auto),
            (" ON ", Mode::On),
            ("true", Mode::On),
            ("0", Mode::Off),
            ("Off", Mode::Off),
            ("", Mode::Auto),
        ] {
            assert_eq!(Mode::parse(s), Ok(m), "{s:?}");
        }
        assert!(Mode::parse("sometimes").unwrap_err().contains("auto, on, off"));
    }

    #[test]
    fn undo_steps_are_labelled_with_tool_and_action() {
        assert_eq!(label_for("draw_shape", &json!({ "shape": "rectangle", "from": [0, 0] })), "draw_shape rectangle");
        assert_eq!(label_for("layer", &json!({ "action": "add" })), "layer add");
        assert_eq!(label_for("fill", &json!({ "x": 1 })), "fill");
    }

    #[test]
    fn app_ids_carry_the_app_prefix() {
        assert_eq!(client::strip_prefix(" app:doc-3 "), Some("doc-3"));
        assert_eq!(client::strip_prefix("d1"), None);
    }
}
