# Contributing to pixmith-mcp

Thanks for helping. This server is a thin layer over [`pixmith-core`](https://github.com/azdharsyahputra/pixmith); the
pixel work happens there, and every tool calls the same `pixmith_*` entry points as the Pixmith apps.

## Build and test

```sh
cargo build
cargo test                               # unit tests + the binary driven over stdio (tests/*.rs)
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
```

The tests start the real binary in a temporary directory (also used as `HOME`), so they never touch your files or
any client configuration.

To work against a local checkout of Pixmith instead of the pinned git revision, uncomment the `[patch]` section at the
end of `Cargo.toml` (it expects the repository next to this one, at `../pixa`).

## Layout

- `src/main.rs`: command line (`--root`, `--http`, `print-config`, `list-tools`) and transports.
- `src/server.rs`: the MCP handler (tools, prompts, resources).
- `src/engine.rs`: the engine thread that owns every document (the core keeps per-thread state), document lookup,
  `Ref` (index or name) resolution, and `DocEntry::edit`, which turns every mutating call into one labeled undo step.
- `src/tools/*.rs`: one module per tool group. Argument structs double as the JSON Schemas (`schemars`); keep field
  doc comments precise, they are what the model reads.
- `src/live/`: live mode. `discovery.rs` (the app's `Live/*.json` files), `client.rs` (the JSON-RPC connection),
  `mod.rs` (shadow documents, routing between app and headless, apply with stale/busy retries). Tools stay unaware of
  it: they run on a decoded copy of the app's document and the result is applied back. `tests/common/mock_app.rs` is a
  mock of the app's server (real `pixmith-core` editors) that `tests/live.rs` drives the binary against.
- `src/paths.rs`: the `--root` sandbox. Every path a tool reads or writes must go through `Sandbox::read`,
  `Sandbox::dir` or `Sandbox::write`.

## Adding a tool or a group

1. Add a handler `fn(&mut Engine, Args) -> R<Out>` and list it with `tool!(...)` in the module's `tools()`; a new
   group is a new module listed in `tools::all()` (a `project_*` group for Pixmith projects would go in
   `src/tools/project.rs`).
2. Mutations go through `DocEntry::edit` (or `edit_changed`) so they are undoable and failures leave no trace.
3. Pick honest hints (`READ`, `ADD`, `CHANGE`, `SET`, `WRITE_FILES`, ...).
4. Cover it in `tests/features.rs` and regenerate the README table with `cargo run -- list-tools --markdown`.

Commits follow Conventional Commits (`feat:`, `fix:`, `docs:`, ...).
