#!/usr/bin/env node
// Runs the prebuilt pixmith-mcp binary from the platform package npm installed next to this one
// (@pixmith/mcp-<platform>-<arch>, an optional dependency), with the same arguments and stdio.
"use strict";

const { spawn } = require("child_process");
const path = require("path");

const exe = process.platform === "win32" ? "pixmith-mcp.exe" : "pixmith-mcp";

function binaryPath() {
  if (process.env.PIXMITH_MCP_BINARY) return process.env.PIXMITH_MCP_BINARY;
  const pkg = `@pixmith/mcp-${process.platform}-${process.arch}`;
  try {
    return path.join(path.dirname(require.resolve(`${pkg}/package.json`)), "bin", exe);
  } catch {
    process.stderr.write(
      `pixmith-mcp: no prebuilt binary for ${process.platform}-${process.arch} (${pkg} is not installed).\n` +
        "Supported: darwin-arm64, darwin-x64, linux-x64, linux-arm64, win32-x64. If yours is one of them, reinstall\n" +
        "without --no-optional / --omit=optional. Otherwise build from source:\n" +
        "  cargo install --git https://github.com/azdharsyahputra/pixmith-mcp\n",
    );
    process.exit(1);
  }
}

const child = spawn(binaryPath(), process.argv.slice(2), { stdio: "inherit", windowsHide: true });
child.on("error", (e) => {
  process.stderr.write(`pixmith-mcp: cannot start ${e.path || "the binary"}: ${e.message}\n`);
  process.exit(1);
});
for (const sig of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(sig, () => child.kill(sig));
}
child.on("exit", (code, signal) => {
  if (signal) {
    process.removeAllListeners(signal);
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});
