//! The engine thread. The core keeps per-thread state (last error, ink and brush settings, running strokes,
//! the cel and tween clipboards) and a document must only be used by one thread at a time, so every
//! document lives on one dedicated thread and every tool call runs there, one after another.

use crate::doc::{Doc, take_string};
use crate::live::{Live, Mode};
use crate::paths::Sandbox;
use pixmith_core::document::{layers as layers_ffi, slices as slices_ffi, tilemap as tm};
use pixmith_core::ffi::document as core;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::mpsc;

pub type R<T> = Result<T, String>;

/// Pixels copied with the `clipboard` tool, shared by all documents.
pub struct Clip {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// An agent-opened transaction (`transaction` tool).
pub struct Txn {
    pub label: String,
    pub undo_at_begin: u32,
}

pub struct DocEntry {
    pub id: String,
    pub name: String,
    pub doc: Doc,
    /// Where it was opened from or last saved to (only native documents: .pxmt / .aseprite).
    pub path: Option<PathBuf>,
    /// Where it was imported from (PNG, GIF, sheet).
    pub source: Option<PathBuf>,
    /// Defaults for tools called without `layer` / `frame`.
    pub layer: u32,
    pub frame: u32,
    pub txns: Vec<Txn>,
    /// The layer and frame the running tool asked for by name or index (live mode moves the app's current layer
    /// and frame there). Reset by whoever starts a tool call.
    pub asked: Cell<(Option<u32>, Option<u32>)>,
}

/// How an edit gets its undo step.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Undo {
    /// Open a step first (`pixmith_begin_stroke`) inside a transaction, so the edit is exactly one undo step
    /// whether or not the core call takes its own checkpoint.
    Step,
    /// The call continues a session the core tracks itself (floating paste, free transform, undo/redo):
    /// opening a step would end that session.
    Live,
}

impl DocEntry {
    /// Runs one mutating operation as a single labeled undo step. On failure, a step the operation opened is
    /// rolled back so a failed tool call leaves no trace.
    pub fn edit<T>(&mut self, label: &str, mode: Undo, f: impl FnOnce(&mut DocEntry) -> R<T>) -> R<T> {
        crate::doc::clear_error();
        if mode == Undo::Live {
            let out = f(self);
            if out.is_ok() {
                self.doc.touch();
            }
            return out;
        }
        let (before, _) = self.doc.history_counts();
        self.doc.begin_transaction();
        self.doc.begin_stroke();
        let out = f(self);
        self.doc.end_transaction();
        let (after, _) = self.doc.history_counts();
        match out {
            Ok(v) => {
                // Inside an agent transaction the transaction's own label names the step.
                if after > before && self.txns.is_empty() {
                    self.doc.label(label);
                }
                self.doc.touch();
                Ok(v)
            }
            Err(e) => {
                if after > before {
                    self.doc.rollback();
                }
                Err(e)
            }
        }
    }

    /// Like [`edit`](Self::edit) for operations that report whether anything changed: when they report
    /// `false`, the undo step they would leave is dropped.
    pub fn edit_changed(&mut self, label: &str, f: impl FnOnce(&mut DocEntry) -> R<bool>) -> R<bool> {
        let (before, _) = self.doc.history_counts();
        let changed = self.edit(label, Undo::Step, f)?;
        if !changed && self.doc.history_counts().0 > before {
            self.doc.rollback();
        }
        Ok(changed)
    }

    pub fn layer_count(&self) -> u32 {
        self.doc.layer_count()
    }

    pub fn layer_name(&self, i: u32) -> String {
        unsafe { take_string(core::pixmith_layer_name(self.doc.raw(), i)) }
    }

    pub fn is_group(&self, i: u32) -> bool {
        unsafe { core::pixmith_layer_is_group(self.doc.raw(), i) }
    }

    pub fn is_tilemap(&self, i: u32) -> bool {
        unsafe { tm::pixmith_layer_is_tilemap(self.doc.raw(), i) }
    }

    /// A layer index from a reference (default: the active layer).
    pub fn layer(&self, r: &Option<Ref>) -> R<u32> {
        let n = self.layer_count();
        let i = match r {
            None => self.layer.min(n.saturating_sub(1)),
            Some(r) => {
                let i = r.resolve("layer", n, |i| self.layer_name(i))?;
                self.asked.set((Some(i), self.asked.get().1));
                i
            }
        };
        if i >= n {
            return Err(format!("layer {i} does not exist (the document has {n} layers, 0 = bottom)"));
        }
        Ok(i)
    }

    /// A layer that holds pixels (not a group).
    pub fn image_layer(&self, r: &Option<Ref>) -> R<u32> {
        let i = self.layer(r)?;
        if self.is_group(i) {
            return Err(format!(
                "layer {i} ({:?}) is a group; pick a layer inside it (see doc_info)",
                self.layer_name(i)
            ));
        }
        Ok(i)
    }

    pub fn tilemap_layer(&self, r: &Option<Ref>) -> R<u32> {
        let i = self.layer(r)?;
        if !self.is_tilemap(i) {
            return Err(format!(
                "layer {i} ({:?}) is not a tilemap layer; create one with tilemap action add_layer or convert",
                self.layer_name(i)
            ));
        }
        Ok(i)
    }

    /// A frame index (default: the active frame).
    pub fn frame(&self, f: Option<u32>) -> R<u32> {
        let n = self.doc.frame_count();
        let explicit = f.is_some();
        let f = f.unwrap_or(self.frame.min(n.saturating_sub(1)));
        if f >= n {
            return Err(format!("frame {f} does not exist (the document has {n} frames, 0-based)"));
        }
        if explicit {
            self.asked.set((self.asked.get().0, Some(f)));
        }
        Ok(f)
    }

    pub fn tag(&self, r: &Ref) -> R<u32> {
        let n = unsafe { layers_ffi::pixmith_tag_count(self.doc.raw()) };
        r.resolve("tag", n, |i| unsafe { take_string(layers_ffi::pixmith_tag_name(self.doc.raw(), i)) })
    }

    pub fn slice(&self, r: &Ref) -> R<u32> {
        let n = unsafe { slices_ffi::pixmith_slice_count(self.doc.raw()) };
        r.resolve("slice", n, |i| unsafe { take_string(slices_ffi::pixmith_slice_name(self.doc.raw(), i)) })
    }

    pub fn tileset(&self, r: &Ref) -> R<u32> {
        let n = unsafe { tm::pixmith_tileset_count(self.doc.raw()) };
        r.resolve("tileset", n, |i| unsafe { take_string(tm::pixmith_tileset_name(self.doc.raw(), i)) })
    }

    /// Keeps the defaults valid after layers or frames were removed.
    pub fn clamp_active(&mut self) {
        self.layer = self.layer.min(self.layer_count().saturating_sub(1));
        self.frame = self.frame.min(self.doc.frame_count().saturating_sub(1));
    }
}

/// A layer, tag, slice or tileset given by 0-based index or by name.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Ref {
    Index(u32),
    Name(String),
}

impl Ref {
    pub fn resolve(&self, what: &str, count: u32, name: impl Fn(u32) -> String) -> R<u32> {
        match self {
            Ref::Index(i) => {
                if *i < count {
                    Ok(*i)
                } else {
                    Err(format!("{what} {i} does not exist (there are {count}, 0-based)"))
                }
            }
            Ref::Name(n) => {
                if let Ok(i) = n.trim().parse::<u32>() {
                    return Ref::Index(i).resolve(what, count, name);
                }
                let names: Vec<String> = (0..count).map(&name).collect();
                let exact: Vec<u32> = (0..count).filter(|&i| names[i as usize] == *n).collect();
                let hits = if exact.is_empty() {
                    (0..count).filter(|&i| names[i as usize].eq_ignore_ascii_case(n.trim())).collect()
                } else {
                    exact
                };
                match hits.as_slice() {
                    [i] => Ok(*i),
                    [] => Err(format!("no {what} named {n:?} (names: {})", list_names(&names))),
                    many => Err(format!("{many:?} are all {what}s named {n:?}; use the index instead")),
                }
            }
        }
    }
}

fn list_names(names: &[String]) -> String {
    if names.is_empty() {
        return "none".into();
    }
    let shown: Vec<String> = names.iter().take(40).enumerate().map(|(i, n)| format!("{i}={n:?}")).collect();
    let more = if names.len() > 40 { format!(", ... {} more", names.len() - 40) } else { String::new() };
    format!("{}{more}", shown.join(", "))
}

impl JsonSchema for Ref {
    fn schema_name() -> Cow<'static, str> {
        "Ref".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "anyOf": [{ "type": "integer", "minimum": 0 }, { "type": "string" }],
            "description": "0-based index or exact name"
        })
    }
}

pub struct Engine {
    pub docs: Vec<DocEntry>,
    next_id: u32,
    pub sandbox: Sandbox,
    pub clip: Option<Clip>,
    /// Size of the block in the cel clipboard (`cel copy`): layers, frames and how many of its cels hold an image.
    pub cel_clip: Option<(u32, u32, u32)>,
    /// Isometric grid settings per document id (`iso_grid` tool), the default of `select` action `iso_tile`.
    pub iso_grids: HashMap<String, crate::iso::Iso>,
    /// The connection to a running Pixmith app (live mode).
    pub live: Live,
}

impl Engine {
    pub fn new(sandbox: Sandbox, live: Mode) -> Engine {
        Engine {
            docs: Vec::new(),
            next_id: 1,
            sandbox,
            clip: None,
            cel_clip: None,
            iso_grids: HashMap::new(),
            live: Live::new(live),
        }
    }

    /// Removes and returns the newest document, giving its id back for the next one. Live mode builds some
    /// documents here and moves them into the app.
    pub fn take_last(&mut self) -> Option<DocEntry> {
        let d = self.docs.pop()?;
        if d.id == format!("d{}", self.next_id - 1) {
            self.next_id -= 1;
        }
        Some(d)
    }

    /// Headless documents kept inside the server (not the shadows of app documents).
    pub fn headless_count(&self) -> usize {
        self.docs.iter().filter(|d| !d.id.starts_with("app:")).count()
    }

    /// Closes the oldest unmodified headless documents (saved or never edited, no open transaction) until at most
    /// `max` are left, never `keep`. Returns the closed ids, oldest first.
    pub fn trim_headless(&mut self, max: usize, keep: &str) -> Vec<String> {
        let mut excess = self.headless_count().saturating_sub(max);
        let mut closed = Vec::new();
        self.docs.retain(|d| {
            let closable =
                excess > 0 && d.id != keep && !d.id.starts_with("app:") && !d.doc.is_dirty() && d.txns.is_empty();
            if closable {
                excess -= 1;
                closed.push(d.id.clone());
            }
            !closable
        });
        closed
    }

    /// Registers a document and returns its id.
    pub fn add(&mut self, doc: Doc, name: String, path: Option<PathBuf>, source: Option<PathBuf>) -> String {
        let id = format!("d{}", self.next_id);
        self.next_id += 1;
        self.docs.push(DocEntry {
            id: id.clone(),
            name,
            doc,
            path,
            source,
            layer: 0,
            frame: 0,
            txns: Vec::new(),
            asked: Cell::new((None, None)),
        });
        id
    }

    /// The document `id`, or the only open document when `id` is omitted.
    pub fn doc(&mut self, id: &Option<String>) -> R<&mut DocEntry> {
        match id {
            Some(id) => {
                let ids = self.ids();
                let id = id.trim();
                self.docs
                    .iter_mut()
                    .find(|d| d.id == id || d.name == id)
                    .ok_or_else(|| format!("no open document {id:?} (open: {ids})"))
            }
            None => match self.docs.len() {
                0 => Err("no document is open; call doc_new or doc_open first".into()),
                1 => Ok(&mut self.docs[0]),
                _ => Err(format!("several documents are open ({}); pass `doc`", self.ids())),
            },
        }
    }

    fn ids(&self) -> String {
        if self.docs.is_empty() {
            return "none".into();
        }
        self.docs.iter().map(|d| format!("{} ({})", d.id, d.name)).collect::<Vec<_>>().join(", ")
    }
}

type Job = Box<dyn FnOnce(&mut Engine) + Send>;

/// Sends work to the engine thread.
pub struct EngineHandle {
    tx: mpsc::Sender<Job>,
}

impl EngineHandle {
    pub fn spawn(sandbox: Sandbox, live: Mode) -> EngineHandle {
        let (tx, rx) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("pixmith-engine".into())
            .stack_size(16 << 20)
            .spawn(move || {
                let mut engine = Engine::new(sandbox, live);
                while let Ok(job) = rx.recv() {
                    // A bug in a tool must not take the documents down with it; the core guards its own calls.
                    let _ = catch_unwind(AssertUnwindSafe(|| job(&mut engine)));
                }
            })
            .expect("spawn the engine thread");
        EngineHandle { tx }
    }

    /// Runs `f` on the engine thread and waits for its result.
    pub async fn run<T: Send + 'static>(&self, f: impl FnOnce(&mut Engine) -> T + Send + 'static) -> R<T> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.tx
            .send(Box::new(move |e| {
                let _ = tx.send(f(e));
            }))
            .map_err(|_| "the engine thread stopped".to_string())?;
        rx.await.map_err(|_| "internal error: the tool panicked (the documents are intact)".to_string())
    }
}
