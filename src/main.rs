//! pixmith-mcp: a Model Context Protocol server that gives AI agents the full Pixmith pixel-art editor.

// `chunks_exact(4)` over RGBA bytes is the idiom of the Pixmith core as well; `as_chunks` would only add noise.
#![allow(clippy::chunks_exact_to_as_chunks)]

mod color;
mod config;
mod doc;
mod engine;
mod image;
mod iso;
mod live;
mod paths;
mod schema;
mod server;
mod tools;

use engine::EngineHandle;
use rmcp::ServiceExt;
use server::PixmithServer;
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str = "pixmith-mcp: MCP server for the Pixmith pixel-art editor

USAGE:
    pixmith-mcp [--root <dir>] [--allow-dir <dir>]... [--http <addr>] [--live <mode>]
                                                   run the server (stdio by default)
    pixmith-mcp print-config <client> [--root <dir>] [--allow-dir <dir>]... [--live <mode>] [--name <name>]
                                                   print the config snippet for an MCP client
    pixmith-mcp list-tools [--markdown]            print the tool catalogue
    pixmith-mcp --version | --help

OPTIONS:
    --root <dir>    only read and write files inside <dir> (symlinks resolved); relative paths start there
    --allow-dir <dir>  (repeatable) with --root: also allow reading files in <dir> and writing `export` output there
                    (e.g. a temp or scratch folder); saving documents and other writes stay inside the root
    --http <addr>   serve Streamable HTTP at http://<addr>/mcp instead of stdio (e.g. 127.0.0.1:8765)
    --live <mode>   auto (default): edit the documents open in the Pixmith app when it is running on this computer,
                    headless otherwise; on: require the app (tools fail when none is running); off: headless only.
                    The PIXMITH_LIVE environment variable sets the same; the flag wins.

CLIENTS for print-config:
";

struct Opts {
    root: Option<PathBuf>,
    allow_dirs: Vec<PathBuf>,
    http: Option<String>,
    live: Option<String>,
    name: String,
    markdown: bool,
    rest: Vec<String>,
}

fn parse(args: Vec<String>) -> Result<Opts, String> {
    let mut o = Opts {
        root: None,
        allow_dirs: Vec::new(),
        http: None,
        live: None,
        name: "pixmith".into(),
        markdown: false,
        rest: Vec::new(),
    };
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let mut value = |flag: &str| it.next().ok_or_else(|| format!("{flag} needs a value"));
        match a.as_str() {
            "--root" => o.root = Some(PathBuf::from(value("--root")?)),
            "--allow-dir" => o.allow_dirs.push(PathBuf::from(value("--allow-dir")?)),
            "--http" => o.http = Some(value("--http")?),
            "--live" => o.live = Some(value("--live")?),
            "--name" => o.name = value("--name")?,
            "--markdown" => o.markdown = true,
            s if s.starts_with("--root=") => o.root = Some(PathBuf::from(&s[7..])),
            s if s.starts_with("--allow-dir=") => o.allow_dirs.push(PathBuf::from(&s[12..])),
            s if s.starts_with("--http=") => o.http = Some(s[7..].to_string()),
            s if s.starts_with("--live=") => o.live = Some(s[7..].to_string()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ => o.rest.push(a),
        }
    }
    Ok(o)
}

fn usage() -> String {
    let mut s = USAGE.to_string();
    for (c, d) in config::CLIENTS {
        s.push_str(&format!("    {c:<16}{d}\n"));
    }
    s
}

fn list_tools(markdown: bool) {
    let all = tools::all();
    if markdown {
        println!("| Group | Tool | What it does | Hints |\n| --- | --- | --- | --- |");
    }
    for t in &all {
        let first = t.description.split(". ").next().unwrap_or(t.description).trim_end_matches('.');
        let h = t.hints;
        let hints = if h.read_only {
            "read-only".to_string()
        } else {
            [h.destructive.then_some("destructive"), h.idempotent.then_some("idempotent"), h.files.then_some("files")]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ")
        };
        if markdown {
            println!("| {} | `{}` | {} | {} |", t.group, t.name, first.replace('|', "\\|"), hints);
        } else {
            println!("{:<22} {}", t.name, first);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", usage());
        return;
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("pixmith-mcp {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let opts = match parse(args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("pixmith-mcp: {e}\n\n{}", usage());
            std::process::exit(2);
        }
    };
    match opts.rest.first().map(String::as_str) {
        Some("print-config") => {
            let Some(client) = opts.rest.get(1) else {
                eprintln!("pixmith-mcp: print-config needs a client\n\n{}", usage());
                std::process::exit(2);
            };
            let exe = std::env::current_exe()
                .ok()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| paths::show(&p))
                .unwrap_or_else(|| "pixmith-mcp".into());
            let mut args = Vec::new();
            if let Some(r) = &opts.root {
                let abs =
                    r.canonicalize().map(|p| paths::show(&p)).unwrap_or_else(|_| r.to_string_lossy().into_owned());
                args.extend(["--root".to_string(), abs]);
            }
            for d in &opts.allow_dirs {
                let abs =
                    d.canonicalize().map(|p| paths::show(&p)).unwrap_or_else(|_| d.to_string_lossy().into_owned());
                args.extend(["--allow-dir".to_string(), abs]);
            }
            if let Some(l) = &opts.live {
                if let Err(e) = live::Mode::parse(l) {
                    eprintln!("pixmith-mcp: {e}");
                    std::process::exit(2);
                }
                args.extend(["--live".to_string(), l.trim().to_ascii_lowercase()]);
            }
            match config::print(client, &exe, &args, &opts.name) {
                Ok(s) => println!("{s}"),
                Err(e) => {
                    eprintln!("pixmith-mcp: {e}");
                    std::process::exit(2);
                }
            }
        }
        Some("list-tools") => list_tools(opts.markdown),
        Some(other) => {
            eprintln!("pixmith-mcp: unknown command {other:?}\n\n{}", usage());
            std::process::exit(2);
        }
        None => {
            if let Err(e) = serve(opts) {
                eprintln!("pixmith-mcp: {e}");
                std::process::exit(1);
            }
        }
    }
}

fn serve(opts: Opts) -> Result<(), String> {
    let sandbox = paths::Sandbox::new(opts.root.as_deref(), &opts.allow_dirs)?;
    let mode = match (&opts.live, std::env::var("PIXMITH_LIVE")) {
        (Some(flag), _) => live::Mode::parse(flag).map_err(|e| format!("--live: {e}"))?,
        (None, Ok(var)) => live::Mode::parse(&var).map_err(|e| format!("PIXMITH_LIVE: {e}"))?,
        (None, Err(_)) => live::Mode::Auto,
    };
    let engine = Arc::new(EngineHandle::spawn(sandbox, mode));
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async move {
        match opts.http {
            Some(addr) => serve_http(engine, &addr).await,
            None => {
                let server = PixmithServer::new(engine);
                let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| format!("cannot start: {e}"))?;
                running.waiting().await.map_err(|e| e.to_string())?;
                Ok(())
            }
        }
    })
}

#[cfg(feature = "http")]
async fn serve_http(engine: Arc<EngineHandle>, addr: &str) -> Result<(), String> {
    use rmcp::transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    };
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    // Only local hosts and local browser origins by default (DNS rebinding / drive-by protection).
    let mut hosts = vec!["localhost".to_string(), "127.0.0.1".to_string(), "::1".to_string()];
    if !local.ip().is_loopback() {
        eprintln!("pixmith-mcp: warning: {local} is reachable from other machines and there is no authentication");
        hosts.push(local.ip().to_string());
    }
    let config = StreamableHttpServerConfig::default().with_allowed_hosts(hosts).with_allowed_origins([
        "http://localhost:*",
        "http://127.0.0.1:*",
        "http://[::1]:*",
    ]);
    let server = PixmithServer::new(engine);
    let service: StreamableHttpService<PixmithServer, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    let router = axum::Router::new().nest_service("/mcp", service);
    eprintln!("pixmith-mcp: serving MCP (Streamable HTTP) at http://{local}/mcp");
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .map_err(|e| e.to_string())
}

#[cfg(not(feature = "http"))]
async fn serve_http(_: Arc<EngineHandle>, _: &str) -> Result<(), String> {
    Err("this build has no HTTP transport (build with the `http` feature)".into())
}
