//! Live mode controls: is the Pixmith app connected, connect to it, disconnect from it.

use super::docs::NoArgs;
use super::{Hints, Out, READ, ToolDef, tool};
use crate::engine::{Engine, R};
use crate::image::decode_png;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

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
        tool!(
            "App",
            "game_screenshot",
            "Screenshot of the running Godot game",
            READ,
            ScreenshotArgs,
            game_screenshot,
            "Take a screenshot of the running Godot game and return it as an image (plus its size and the game's client name as JSON). Needs live mode (Pixmith 0.5.1 or later running) and a game started from Godot in debug with the Pixmith Live Link addon enabled (see export_godot). After editing a sprite, call this to see the result in the game itself: Live Link has already put the new art into the running game. Pixel-art games are small, so `scale` (integer 1-8, nearest neighbour, long side capped at 2048 px) enlarges the picture. Errors when no such game is running or it does not answer within `timeout_ms`."
        ),
    ]
}

#[derive(Deserialize, JsonSchema)]
pub struct ScreenshotArgs {
    /// How long to wait for the game's frame, in milliseconds (default 3000, at most 10000).
    pub timeout_ms: Option<u64>,
    /// Integer upscale 1-8 for small pixel games (default 1); nearest neighbour, the long side stays within 2048 px.
    pub scale: Option<u32>,
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

/// Longest side of an enlarged screenshot.
const MAX_SIDE: usize = 2048;

fn game_screenshot(e: &mut Engine, a: ScreenshotArgs) -> R<Out> {
    let scale = a.scale.unwrap_or(1);
    if !(1..=8).contains(&scale) {
        return Err(format!("scale must be 1-8, got {scale}"));
    }
    let timeout = a.timeout_ms.unwrap_or(3000).clamp(1, 10_000);
    let (png, w, h, client) = e.live.game_capture(timeout)?;
    let mut value = json!({ "w": w, "h": h, "client": client });
    if scale == 1 {
        return Ok(Out { value, images: vec![png], ..Default::default() });
    }
    let img = decode_png(&png).map_err(|err| format!("the game sent a PNG that cannot be read: {err}"))?;
    let scale = (scale as usize).min(MAX_SIDE / img.w.max(img.h).max(1)).max(1);
    let img = img.upscale(scale);
    value = json!({ "w": img.w, "h": img.h, "client": client, "scale": scale });
    Ok(Out { value, images: vec![img.png()?], ..Default::default() })
}
