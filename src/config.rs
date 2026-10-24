//! `pixmith-mcp print-config <client>`: the configuration snippet each MCP client expects, filled in with
//! this binary's absolute path. Nothing is written; paste the output where the comment says.

use serde_json::json;

pub const CLIENTS: &[(&str, &str)] = &[
    ("claude-code", "Claude Code (CLI command)"),
    ("claude-desktop", "Claude Desktop (claude_desktop_config.json)"),
    ("codex", "OpenAI Codex CLI (~/.codex/config.toml)"),
    ("antigravity", "Google Antigravity (mcp_config.json)"),
    ("cursor", "Cursor (.cursor/mcp.json)"),
    ("gemini", "Gemini CLI (settings.json)"),
    ("vscode", "VS Code / GitHub Copilot (.vscode/mcp.json)"),
    ("windsurf", "Windsurf (mcp_config.json)"),
    ("json", "Generic mcpServers JSON (Cline, Roo, Zed, LM Studio, ...)"),
];

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn shell(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-:=+,@".contains(c)) {
        s.to_string()
    } else if cfg!(windows) {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

pub fn print(client: &str, exe: &str, args: &[String], name: &str) -> Result<String, String> {
    let servers = json!({ name: { "command": exe, "args": args } });
    let pretty = |v: serde_json::Value| serde_json::to_string_pretty(&v).unwrap_or_default();
    let cmdline =
        std::iter::once(exe.to_string()).chain(args.iter().cloned()).map(|a| shell(&a)).collect::<Vec<_>>().join(" ");
    Ok(match client {
        "claude-code" | "claude" => format!(
            "# Run in a terminal (add --scope user to make it available in every project, --scope project to share it via .mcp.json):\nclaude mcp add {name} -- {cmdline}\n\n# Or put this in .mcp.json at the project root:\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "claude-desktop" => format!(
            "// Claude Desktop: Settings > Developer > Edit Config, or edit\n//   macOS:   ~/Library/Application Support/Claude/claude_desktop_config.json\n//   Windows: %APPDATA%\\Claude\\claude_desktop_config.json\n// Merge into \"mcpServers\", then restart Claude Desktop.\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "codex" => format!(
            "# Codex CLI: add to ~/.codex/config.toml (or run: codex mcp add {name} -- {cmdline})\n[mcp_servers.{name}]\ncommand = {}\nargs = [{}]\n",
            toml_str(exe),
            args.iter().map(|a| toml_str(a)).collect::<Vec<_>>().join(", ")
        ),
        "antigravity" => format!(
            "// Antigravity: ~/.gemini/config/mcp_config.json (all projects) or <workspace>/.agents/mcp_config.json\n//   (Agent panel > ... > MCP Servers > Manage MCP Servers > View raw config). Merge into \"mcpServers\".\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "cursor" => format!(
            "// Cursor: .cursor/mcp.json in the project, or ~/.cursor/mcp.json for all projects.\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "gemini" => format!(
            "// Gemini CLI: ~/.gemini/settings.json (user) or .gemini/settings.json (project). Merge into \"mcpServers\".\n// Or run: gemini mcp add {name} {cmdline}\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "vscode" | "copilot" => format!(
            "// VS Code (Copilot agent mode): .vscode/mcp.json in the workspace, or \"MCP: Open User Configuration\".\n// Note: VS Code uses \"servers\", not \"mcpServers\".\n{}",
            pretty(json!({ "servers": { name: { "type": "stdio", "command": exe, "args": args } } }))
        ),
        "windsurf" => format!(
            "// Windsurf: Cascade > MCP servers > Manage > View raw config (~/.codeium/windsurf/mcp_config.json).\n{}",
            pretty(json!({ "mcpServers": servers }))
        ),
        "json" => pretty(json!({ "mcpServers": servers })),
        other => {
            return Err(format!(
                "unknown client {other:?}; one of: {}",
                CLIENTS.iter().map(|(c, _)| *c).collect::<Vec<_>>().join(", ")
            ));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_client_prints_valid_snippets() {
        let args = vec!["--root".to_string(), "/tmp/my art".to_string()];
        for (c, _) in CLIENTS {
            let s = print(c, "/opt/pixmith-mcp", &args, "pixmith").unwrap();
            assert!(s.contains("/opt/pixmith-mcp"), "{c}");
            if let Some(start) = s.find('{')
                && *c != "codex"
                && *c != "claude-code"
            {
                let v: serde_json::Value =
                    serde_json::from_str(&s[start..]).unwrap_or_else(|e| panic!("{c}: {e}\n{s}"));
                assert!(v.is_object());
            }
        }
        let codex = print("codex", "C:\\bin\\pixmith-mcp.exe", &args, "pixmith").unwrap();
        assert!(codex.contains("command = \"C:\\\\bin\\\\pixmith-mcp.exe\""));
        assert!(codex.contains("[mcp_servers.pixmith]"));
        assert!(print("nope", "x", &[], "pixmith").is_err());
    }
}
