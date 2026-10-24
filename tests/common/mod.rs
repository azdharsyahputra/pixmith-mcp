//! A minimal MCP client for the tests: spawns the real binary and speaks JSON-RPC over its stdio.
#![allow(dead_code)]

use base64::Engine as _;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct Client {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    next: u64,
    pub dir: tempfile::TempDir,
}

impl Client {
    /// Starts the server in a fresh temporary directory (also its HOME, so nothing touches the real one).
    pub fn start(extra: &[&str]) -> Client {
        let dir = tempfile::tempdir().expect("tempdir");
        Client::start_in(dir, extra)
    }

    pub fn start_in(dir: tempfile::TempDir, extra: &[&str]) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pixmith-mcp"))
            .args(extra)
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("APPDATA", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn pixmith-mcp");
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut c = Client { child, stdin, out, next: 1, dir };
        let init = c.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "pixmith-mcp-tests", "version": "1" } }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "pixmith-mcp", "{init}");
        c.notify("notifications/initialized", json!({}));
        c
    }

    pub fn path(&self, p: &str) -> PathBuf {
        self.dir.path().join(p)
    }

    pub fn p(&self, p: &str) -> String {
        self.path(p).to_string_lossy().into_owned()
    }

    fn send(&mut self, v: &Value) {
        writeln!(self.stdin, "{v}").unwrap();
        self.stdin.flush().unwrap();
    }

    pub fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    /// Sends a request and returns the whole response object.
    pub fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            let n = self.out.read_line(&mut line).expect("read");
            assert!(n > 0, "server closed stdout while waiting for {method}");
            let v: Value =
                serde_json::from_str(&line).unwrap_or_else(|e| panic!("non-JSON line on stdout ({e}): {line}"));
            if v["id"] == json!(id) {
                return v;
            }
        }
    }

    /// Calls a tool; returns the raw result.
    pub fn call(&mut self, name: &str, args: Value) -> Value {
        let r = self.request("tools/call", json!({ "name": name, "arguments": args }));
        assert!(r.get("error").is_none(), "{name}: protocol error {r}");
        r["result"].clone()
    }

    /// Calls a tool that must succeed; returns its JSON payload.
    pub fn ok(&mut self, name: &str, args: Value) -> Value {
        let r = self.call(name, args.clone());
        assert_eq!(r["isError"], json!(false), "{name} {args} failed: {r}");
        let text = r["content"][0]["text"].as_str().unwrap_or("null");
        serde_json::from_str(text).unwrap_or(Value::String(text.to_string()))
    }

    /// Calls a tool that must fail; returns the error text.
    pub fn err(&mut self, name: &str, args: Value) -> String {
        let r = self.call(name, args.clone());
        assert_eq!(r["isError"], json!(true), "{name} {args} should fail: {r}");
        r["content"][0]["text"].as_str().unwrap_or_default().to_string()
    }

    /// The PNG images of a tool result, decoded to (width, height, rgba).
    pub fn images(result: &Value) -> Vec<(u32, u32, Vec<u8>)> {
        result["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["type"] == "image")
            .map(|c| {
                assert_eq!(c["mimeType"], "image/png");
                let bytes = base64::engine::general_purpose::STANDARD.decode(c["data"].as_str().unwrap()).unwrap();
                decode_png(&bytes)
            })
            .collect()
    }

    /// Composited color at (x, y) of the active frame.
    pub fn pixel(&mut self, x: i32, y: i32) -> String {
        self.ok("get_pixel", json!({ "x": x, "y": y }))["color"].as_str().unwrap().to_string()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG");
    let mut dec = png::Decoder::new(std::io::Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND);
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    buf.truncate(info.buffer_size());
    (info.width, info.height, buf)
}

pub fn png_size(path: &Path) -> (u32, u32) {
    let (w, h, _) = decode_png(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    (w, h)
}
