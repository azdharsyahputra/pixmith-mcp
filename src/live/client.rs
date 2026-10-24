//! The wire side of live mode: one TCP connection speaking JSON-RPC 2.0, one request per line.

use super::discovery::{Entry, PROTOCOL};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Longest message the protocol allows.
const MAX_LINE: u64 = 64 << 20;

pub const STALE: i64 = -32009;
pub const BUSY: i64 = -32010;
pub const UNSAVED: i64 = -32011;
pub const UNKNOWN_DOC: i64 = -32004;
/// `game.capture` found no running game, or no frame arrived in time.
pub const NO_GAME: i64 = -32012;
pub const NO_METHOD: i64 = -32601;

#[derive(Debug)]
pub enum RpcError {
    /// The app answered with a JSON-RPC error.
    Rpc { code: i64, message: String },
    /// The connection failed or the app sent something that is not the protocol.
    Io(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Rpc { message, .. } => write!(f, "{message}"),
            RpcError::Io(m) => write!(f, "{m}"),
        }
    }
}

/// What `hello` told us about the app.
#[derive(Clone, Debug)]
pub struct AppInfo {
    pub app: String,
    pub version: String,
    pub front_end: String,
    pub platform: String,
    pub pid: u32,
    pub port: u16,
}

pub struct Conn {
    reader: BufReader<TcpStream>,
    next: u64,
    pub info: AppInfo,
}

impl Conn {
    /// Connects to the app in a discovery file and says hello.
    pub fn connect(entry: &Entry) -> Result<Conn, String> {
        if entry.protocol != PROTOCOL {
            return Err(format!(
                "the app (pid {}) speaks live protocol {}, this pixmith-mcp speaks {PROTOCOL}; update the older one",
                entry.pid, entry.protocol
            ));
        }
        let addr = SocketAddr::from(([127, 0, 0, 1], entry.port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_millis(750))
            .map_err(|e| format!("pid {} does not answer on port {}: {e}", entry.pid, entry.port))?;
        let _ = stream.set_nodelay(true);
        let timeout = Some(Duration::from_secs(60));
        let _ = stream.set_read_timeout(timeout);
        let _ = stream.set_write_timeout(timeout);
        let mut conn = Conn {
            reader: BufReader::new(stream),
            next: 1,
            info: AppInfo {
                app: entry.app.clone(),
                version: entry.version.clone(),
                front_end: entry.front_end.clone(),
                platform: entry.platform.clone(),
                pid: entry.pid,
                port: entry.port,
            },
        };
        let hello = conn
            .request(
                "hello",
                json!({ "token": entry.token, "client": "pixmith-mcp",
                        "client_version": env!("CARGO_PKG_VERSION"), "protocol": PROTOCOL }),
            )
            .map_err(|e| format!("the app (pid {}) refused the connection: {e}", entry.pid))?;
        let text = |k: &str| hello.get(k).and_then(Value::as_str).map(str::to_string);
        if let Some(a) = text("app") {
            conn.info.app = a;
        }
        if let Some(v) = text("version") {
            conn.info.version = v;
        }
        if let Some(f) = text("front_end") {
            conn.info.front_end = f;
        }
        if let Some(p) = text("platform") {
            conn.info.platform = p;
        }
        let proto = hello.get("protocol").and_then(Value::as_u64).unwrap_or(0);
        if proto != u64::from(PROTOCOL) {
            return Err(format!("the app answered with live protocol {proto}, expected {PROTOCOL}"));
        }
        Ok(conn)
    }

    /// One request and its response.
    pub fn request(&mut self, method: &str, params: Value) -> Result<Value, RpcError> {
        let id = self.next;
        self.next += 1;
        let line = serde_json::to_string(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .map_err(|e| RpcError::Io(e.to_string()))?;
        let stream = self.reader.get_mut();
        stream
            .write_all(line.as_bytes())
            .and_then(|_| stream.write_all(b"\n"))
            .and_then(|_| stream.flush())
            .map_err(|e| RpcError::Io(format!("cannot talk to the Pixmith app: {e}")))?;
        let mut buf = Vec::new();
        let n = (&mut self.reader)
            .take(MAX_LINE + 1)
            .read_until(b'\n', &mut buf)
            .map_err(|e| RpcError::Io(format!("no answer from the Pixmith app: {e}")))?;
        if n == 0 {
            return Err(RpcError::Io("the Pixmith app closed the connection".into()));
        }
        if buf.last() != Some(&b'\n') {
            return Err(RpcError::Io("the Pixmith app sent an unterminated or oversized message".into()));
        }
        let v: Value = serde_json::from_slice(&buf)
            .map_err(|e| RpcError::Io(format!("the Pixmith app sent something that is not JSON: {e}")))?;
        if v.get("id") != Some(&json!(id)) {
            return Err(RpcError::Io(format!("the Pixmith app answered out of order (wanted id {id})")));
        }
        if let Some(err) = v.get("error") {
            return Err(RpcError::Rpc {
                code: err.get("code").and_then(Value::as_i64).unwrap_or(-32000),
                message: err.get("message").and_then(Value::as_str).unwrap_or("unknown error").to_string(),
            });
        }
        Ok(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

/// A document of the app (`Doc` in the protocol).
#[derive(Clone, Debug, PartialEq)]
pub struct AppDoc {
    pub id: String,
    pub name: String,
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    pub frames: u32,
    pub layers: u32,
    pub revision: u64,
    pub dirty: bool,
    pub active: bool,
    pub layer: u32,
    pub frame: u32,
}

impl AppDoc {
    pub fn parse(v: &Value) -> Result<AppDoc, RpcError> {
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::Io("the app sent a document without id".into()))?;
        let n = |k: &str| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        Ok(AppDoc {
            id: id.to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or(id).to_string(),
            path: v.get("path").and_then(Value::as_str).map(str::to_string),
            width: n("width") as u32,
            height: n("height") as u32,
            frames: n("frames") as u32,
            layers: n("layers") as u32,
            revision: n("revision"),
            dirty: v.get("dirty").and_then(Value::as_bool).unwrap_or(false),
            active: v.get("active").and_then(Value::as_bool).unwrap_or(false),
            layer: n("layer") as u32,
            frame: n("frame") as u32,
        })
    }

    /// The id tools use for it: `app:<id>`.
    pub fn tool_id(&self) -> String {
        format!("app:{}", self.id)
    }

    pub fn json(&self) -> Value {
        json!({
            "doc": self.tool_id(), "target": "app", "name": self.name, "path": self.path,
            "width": self.width, "height": self.height, "layers": self.layers, "frames": self.frames,
            "dirty": self.dirty, "active": self.active, "active_layer": self.layer, "active_frame": self.frame,
            "revision": self.revision,
        })
    }
}

/// `app:doc-3` -> `doc-3`.
pub fn strip_prefix(id: &str) -> Option<&str> {
    id.trim().strip_prefix("app:")
}
