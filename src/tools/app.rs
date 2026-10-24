//! Live mode controls: is the Pixmith app connected, connect to it, disconnect from it.

use super::docs::NoArgs;
use super::{Hints, Out, READ, ToolDef, tool};
use crate::engine::{Engine, R};
use schemars::JsonSchema;
use serde::Deserialize;

/// Changes only which documents later tool calls address.
const SWITCH: Hints = Hints { read_only: false, destructive: false, idempotent: true, files: false };

pub fn tools() -> Vec<ToolDef> {
    vec![
        tool!(
            "App",
            "app_status",
            "Pixmith app connection",
            READ,
            NoArgs,
            app_status,
            "Tell whether the Pixmith app is running and connected (live mode): app version and front end, its open documents (ids app:<id>) and the active one, plus the headless documents kept inside this server. When connected, tools edit the app's documents and the user sees every edit appear in the window, each one an undo step \"AI: <tool>\"; otherwise they edit headless documents (ids d1, d2, ...). Call it first when unsure where an edit will land."
        ),
        tool!(
            "App",
            "app_connect",
            "Connect to the Pixmith app",
            SWITCH,
            ConnectArgs,
            app_connect,
            "Connect (or reconnect) to a running Pixmith app on this computer, for example after starting it, and return the same information as app_status. With several instances running the newest is used unless `pid` picks one. Errors when no app is running or the app has live mode switched off in its Preferences."
        ),
        tool!(
            "App",
            "app_disconnect",
            "Disconnect from the Pixmith app",
            SWITCH,
            NoArgs,
            app_disconnect,
            "Disconnect from the Pixmith app and work headless until app_connect is called. The app's documents stay open in the app; headless documents (ids d1, d2, ...) are unaffected."
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct ConnectArgs {
    /// Process id of the app instance to use (see app_status); default: the newest running one.
    pub pid: Option<u32>,
}

fn headless_ids(e: &Engine) -> Vec<String> {
    e.docs.iter().filter(|d| !d.id.starts_with("app:")).map(|d| d.id.clone()).collect()
}

fn app_status(e: &mut Engine, _: NoArgs) -> R<Out> {
    let ids = headless_ids(e);
    Out::json(e.live.status(ids))
}

fn app_connect(e: &mut Engine, a: ConnectArgs) -> R<Out> {
    e.live.enable();
    if a.pid.is_some() {
        e.live.disconnect();
    }
    if !e.live.is_connected() {
        e.live.connect(a.pid)?;
    }
    let ids = headless_ids(e);
    Out::json(e.live.status(ids))
}

fn app_disconnect(e: &mut Engine, _: NoArgs) -> R<Out> {
    e.live.disconnect();
    e.live.set_disabled();
    let ids = headless_ids(e);
    Out::json(e.live.status(ids))
}
