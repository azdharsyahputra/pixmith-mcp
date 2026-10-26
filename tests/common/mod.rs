//! A minimal MCP client for the tests: spawns the real binary and speaks JSON-RPC over its stdio.
#![allow(dead_code)]

use base64::Engine as _;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub mod mock_app;

/// A 16x16 round figure on a 8.3 pixel grid with soft edges, noise, a white paper background and a grey shadow:
/// the "pseudo-pixel" art of AI image tools. Returns (width, height, rgba) and the original sprite.
pub fn pseudo_pixel() -> (usize, usize, Vec<u8>, Vec<[u8; 4]>) {
    const N: usize = 16;
    const PITCH: f64 = 8.3;
    const ORIGIN: (f64, f64) = (14.4, 9.7);
    let c = N as f64 / 2.0;
    let inside = |x: i64, y: i64| {
        x >= 0 && y >= 0 && x < N as i64 && y < N as i64 && {
            let (dx, dy) = ((x as f64 + 0.5 - c) / c, (y as f64 + 0.5 - c) / c);
            dx * dx + dy * dy <= 1.0
        }
    };
    let mut sprite = vec![[0u8; 4]; N * N];
    for y in 0..N {
        for x in 0..N {
            if inside(x as i64, y as i64) {
                let edge =
                    [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().any(|&(dx, dy)| !inside(x as i64 + dx, y as i64 + dy));
                sprite[y * N + x] = if edge {
                    [30, 28, 40, 255]
                } else if y > N / 2 {
                    [240, 190, 140, 255]
                } else {
                    [200, 60, 70, 255]
                };
            }
        }
    }
    for (x, y) in [(5, 5), (6, 5), (9, 5), (10, 5), (5, 6), (10, 6)] {
        sprite[y * N + x] = [255, 255, 255, 255];
    }
    let w = (ORIGIN.0 + N as f64 * PITCH + 2.5 * PITCH).ceil() as usize;
    let h = (ORIGIN.1 + N as f64 * PITCH + 3.5 * PITCH).ceil() as usize;
    let (scx, scy) = (ORIGIN.0 + N as f64 * PITCH / 2.0, ORIGIN.1 + N as f64 * PITCH + 0.3 * PITCH);
    let mut img = vec![[255.0f64; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f64 + 0.5, y as f64 + 0.5);
            let r2 = ((fx - scx) / (N as f64 * PITCH * 0.4)).powi(2) + ((fy - scy) / (1.6 * PITCH)).powi(2);
            let mut col = if r2 < 1.0 { [150.0 + 105.0 * r2; 3] } else { [255.0; 3] };
            let (cx, cy) = (((fx - ORIGIN.0) / PITCH).floor(), ((fy - ORIGIN.1) / PITCH).floor());
            if cx >= 0.0 && cy >= 0.0 && (cx as usize) < N && (cy as usize) < N {
                let p = sprite[cy as usize * N + cx as usize];
                if p[3] != 0 {
                    col = [p[0] as f64, p[1] as f64, p[2] as f64];
                }
            }
            img[y * w + x] = col;
        }
    }
    let blur = |src: &[[f64; 3]], horizontal: bool| -> Vec<[f64; 3]> {
        let mut out = src.to_vec();
        for y in 0..h {
            for x in 0..w {
                let at = |d: i64| {
                    let (sx, sy) = if horizontal {
                        ((x as i64 + d).clamp(0, w as i64 - 1), y as i64)
                    } else {
                        (x as i64, (y as i64 + d).clamp(0, h as i64 - 1))
                    };
                    src[sy as usize * w + sx as usize]
                };
                let (a, b, cc) = (at(-1), at(0), at(1));
                out[y * w + x] = [0, 1, 2].map(|i| (a[i] + 2.0 * b[i] + cc[i]) / 4.0);
            }
        }
        out
    };
    let img = blur(&blur(&img, true), false);
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for p in img {
        for ch in p {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let n = (seed % 13) as f64 - 6.0;
            rgba.push((ch + n).round().clamp(0.0, 255.0) as u8);
        }
        rgba.push(255);
    }
    (w, h, rgba, sprite)
}

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
        Client::start_env(dir, extra, &[])
    }

    /// The folder the server looks for a running app in (`PIXMITH_SUPPORT_DIR`), inside the test directory.
    pub fn support(dir: &Path) -> PathBuf {
        dir.join("support")
    }

    /// Like [`start_in`](Self::start_in) with extra environment variables (they win over the defaults).
    pub fn start_env(dir: tempfile::TempDir, extra: &[&str], env: &[(&str, &str)]) -> Client {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_pixmith-mcp"));
        cmd.args(extra)
            .current_dir(dir.path())
            .env("HOME", dir.path())
            .env("USERPROFILE", dir.path())
            .env("APPDATA", dir.path())
            // Never find (or disturb) a Pixmith app that happens to be running on the developer's machine.
            .env("PIXMITH_SUPPORT_DIR", Client::support(dir.path()))
            .env_remove("PIXMITH_LIVE")
            .env_remove("XDG_DATA_HOME");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let mut child = cmd
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

    /// Sends all the calls at once (like a client making parallel tool calls) and returns the raw results in the
    /// order of `calls`, whatever order the server answered in.
    pub fn call_many(&mut self, calls: &[(&str, Value)]) -> Vec<Value> {
        let first = self.next;
        for (name, args) in calls {
            let id = self.next;
            self.next += 1;
            self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": "tools/call",
                               "params": { "name": name, "arguments": args } }));
        }
        let mut results: Vec<Option<Value>> = vec![None; calls.len()];
        while results.iter().any(Option::is_none) {
            let mut line = String::new();
            let n = self.out.read_line(&mut line).expect("read");
            assert!(n > 0, "server closed stdout while waiting for parallel calls");
            let v: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("non-JSON line ({e}): {line}"));
            if let Some(i) =
                v["id"].as_u64().and_then(|id| id.checked_sub(first)).filter(|i| (*i as usize) < calls.len())
            {
                assert!(v.get("error").is_none(), "protocol error {v}");
                results[i as usize] = Some(v["result"].clone());
            }
        }
        results.into_iter().map(Option::unwrap).collect()
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

/// Writes an RGBA PNG (`rgba` is `w * h * 4` bytes).
pub fn write_png(path: &Path, w: u32, h: u32, rgba: &[u8]) {
    let mut enc = png::Encoder::new(std::fs::File::create(path).unwrap(), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

pub fn png_size(path: &Path) -> (u32, u32) {
    let (w, h, _) = decode_png(&std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
    (w, h)
}
