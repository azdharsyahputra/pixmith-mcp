//! Undo, redo, the history list and agent transactions (several tool calls as one undo step).

use super::{Hints, Out, READ, ToolDef, tool};
use crate::doc::take_string;
use crate::engine::{DocEntry, Engine, R, Txn, Undo};
use pixmith_core::ffi::document as core;
use pixmith_core::history;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const UNDO: Hints = Hints { read_only: false, destructive: true, idempotent: false, files: false };
const TXN: Hints = Hints { read_only: false, destructive: false, idempotent: false, files: false };

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "History",
            "undo",
            "Undo",
            UNDO,
            StepsArgs,
            undo,
            "Undo the last `steps` edits (default 1). Every mutating tool call is one undo step (or one per transaction)."
        ),
        tool!(
            "History",
            "redo",
            "Redo",
            UNDO,
            StepsArgs,
            redo,
            "Redo `steps` edits that were undone (default 1). Any new edit after an undo clears the redo steps."
        ),
        tool!(
            "History",
            "history",
            "Undo history",
            READ,
            HistoryArgs,
            history_list,
            "List the undo history: applied steps (oldest first, with labels such as \"Pencil\" or \"Filter: Blur\") followed by undone steps that can be redone, and the current position. Use undo/redo or history_goto to move."
        ),
        tool!(
            "History",
            "history_goto",
            "Jump in history",
            UNDO,
            GotoArgs,
            history_goto,
            "Undo or redo until exactly `position` steps are applied (0 = the document as opened/created; see history)."
        ),
        tool!(
            "History",
            "transaction",
            "Group edits into one undo step",
            TXN,
            TxnArgs,
            transaction,
            "begin: start a transaction so every following tool call on this document becomes part of ONE undo step named `label` (e.g. a whole character drawn with many calls); end: close it. Transactions nest; end every begin. Undoing while a transaction is open is not allowed."
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct StepsArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// How many steps (default 1).
    pub steps: Option<u32>,
}

fn state(d: &DocEntry) -> Value {
    let (u, r) = d.doc.history_counts();
    let label = |i: u32| unsafe { take_string(history::pixmith_history_label(d.doc.raw(), i)) };
    json!({
        "doc": d.id, "undo_steps": u, "redo_steps": r,
        "next_undo": (u > 0).then(|| label(u - 1)),
        "next_redo": (r > 0).then(|| label(u)),
        "dirty": d.doc.is_dirty(),
    })
}

fn no_txn(d: &DocEntry) -> R<()> {
    if d.txns.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "a transaction ({:?}) is open; end it first",
            d.txns.last().map(|t| t.label.clone()).unwrap_or_default()
        ))
    }
}

fn step(e: &mut Engine, a: StepsArgs, redo: bool) -> R<Out> {
    let d = e.doc(&a.doc)?;
    no_txn(d)?;
    let n = a.steps.unwrap_or(1).clamp(1, 10000);
    let raw = d.doc.raw();
    let done = d.edit(if redo { "Redo" } else { "Undo" }, Undo::Live, |_| {
        let mut done = 0;
        for _ in 0..n {
            let ok = unsafe { if redo { core::pixmith_redo(raw) } else { core::pixmith_undo(raw) } };
            if !ok {
                break;
            }
            done += 1;
        }
        Ok(done)
    })?;
    d.clamp_active();
    let mut v = state(d);
    v[if redo { "redone" } else { "undone" }] = json!(done);
    if done == 0 {
        v["note"] = json!(if redo { "nothing to redo" } else { "nothing to undo" });
    }
    Out::json(v)
}

fn undo(e: &mut Engine, a: StepsArgs) -> R<Out> {
    step(e, a, false)
}

fn redo(e: &mut Engine, a: StepsArgs) -> R<Out> {
    step(e, a, true)
}

#[derive(Deserialize, JsonSchema)]
pub struct HistoryArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// First step to list (default: so that the last `limit` steps show).
    pub offset: Option<u32>,
    /// How many steps (default 50, max 500).
    pub limit: Option<u32>,
}

fn history_list(e: &mut Engine, a: HistoryArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    let (u, r) = d.doc.history_counts();
    let total = u + r;
    let limit = a.limit.unwrap_or(50).clamp(1, 500);
    let offset = a.offset.unwrap_or(u.saturating_sub(limit / 2).min(total.saturating_sub(limit))).min(total);
    let steps: Vec<Value> = (offset..(offset + limit).min(total))
        .map(|i| {
            json!({ "index": i, "label": unsafe { take_string(history::pixmith_history_label(d.doc.raw(), i)) }, "applied": i < u })
        })
        .collect();
    let mut v = state(d);
    v["position"] = json!(u);
    v["total"] = json!(total);
    v["steps"] = json!(steps);
    v["note"] = json!("history_goto position=N leaves steps 0..N-1 applied");
    Out::json(v)
}

#[derive(Deserialize, JsonSchema)]
pub struct GotoArgs {
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// Number of applied steps to end up with.
    pub position: u32,
}

fn history_goto(e: &mut Engine, a: GotoArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    no_txn(d)?;
    let (u, r) = d.doc.history_counts();
    if a.position > u + r {
        return Err(format!("position {} is past the end (0-{})", a.position, u + r));
    }
    let raw = d.doc.raw();
    d.edit("History", Undo::Live, |_| {
        unsafe { history::pixmith_history_goto(raw, a.position) }
            .then_some(())
            .ok_or_else(|| "could not move in the history".to_string())
    })?;
    d.clamp_active();
    Out::json(state(d))
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "snake_case")]
pub enum TxnAction {
    Begin,
    End,
}

#[derive(Deserialize, JsonSchema)]
pub struct TxnArgs {
    pub action: TxnAction,
    /// Document id (see doc_list); optional when only one document is open.
    pub doc: Option<String>,
    /// begin: name of the undo step (default "Edit").
    pub label: Option<String>,
}

fn transaction(e: &mut Engine, a: TxnArgs) -> R<Out> {
    let d = e.doc(&a.doc)?;
    match a.action {
        TxnAction::Begin => {
            if d.txns.len() >= 16 {
                return Err("too many nested transactions".into());
            }
            let (u, _) = d.doc.history_counts();
            d.doc.begin_transaction();
            d.txns.push(Txn { label: a.label.clone().unwrap_or_else(|| "Edit".into()), undo_at_begin: u });
        }
        TxnAction::End => {
            let t = d.txns.pop().ok_or("no transaction is open")?;
            d.doc.end_transaction();
            if d.txns.is_empty() && d.doc.history_counts().0 > t.undo_at_begin {
                d.doc.label(&t.label);
            }
        }
    }
    let mut v = state(d);
    v["open_transactions"] = json!(d.txns.iter().map(|t| t.label.clone()).collect::<Vec<_>>());
    Out::json(v)
}
