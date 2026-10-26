// Stages the npm packages: the `pixmith-mcp` launcher plus one `@pixmith/mcp-<platform>-<arch>` package per
// prebuilt binary (the esbuild / Biome layout: npm installs only the one matching the user's os and cpu).
//
//   node npm/build.mjs <out-dir> <platform>-<arch>=<binary> ...
//   node npm/build.mjs dist/npm darwin-arm64=target/release/pixmith-mcp
//
// The version comes from Cargo.toml. Every package lands in <out-dir>/<dir>, ready for `npm pack` / `npm publish`;
// publish the platform packages before `pixmith-mcp` itself.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.dirname(here);

const PLATFORMS = ["darwin-arm64", "darwin-x64", "linux-x64", "linux-arm64", "win32-x64"];

const [out, ...pairs] = process.argv.slice(2);
if (!out) {
  console.error("usage: node npm/build.mjs <out-dir> <platform>-<arch>=<binary> ...");
  process.exit(2);
}
const cargo = fs.readFileSync(path.join(repo, "Cargo.toml"), "utf8");
const version = cargo.match(/^version\s*=\s*"([^"]+)"/m)[1];
const base = JSON.parse(fs.readFileSync(path.join(here, "pixmith-mcp", "package.json"), "utf8"));

fs.rmSync(out, { recursive: true, force: true });

function write(dir, pkg, readme) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "package.json"), JSON.stringify(pkg, null, 2) + "\n");
  fs.writeFileSync(path.join(dir, "README.md"), readme);
  fs.copyFileSync(path.join(repo, "LICENSE"), path.join(dir, "LICENSE"));
}

const root = path.join(out, "pixmith-mcp");
write(
  root,
  { ...base, version, optionalDependencies: Object.fromEntries(PLATFORMS.map((p) => [`@pixmith/mcp-${p}`, version])) },
  fs.readFileSync(path.join(repo, "README.md"), "utf8"),
);
fs.cpSync(path.join(here, "pixmith-mcp", "bin"), path.join(root, "bin"), { recursive: true });
fs.chmodSync(path.join(root, "bin", "pixmith-mcp.js"), 0o755);

for (const pair of pairs) {
  const [target, binary] = pair.split("=");
  if (!PLATFORMS.includes(target) || !binary) {
    console.error(`bad argument ${pair}: want <platform>-<arch>=<binary>, platform one of ${PLATFORMS.join(", ")}`);
    process.exit(2);
  }
  const [os, cpu] = target.split("-");
  const name = `@pixmith/mcp-${target}`;
  const dir = path.join(out, `mcp-${target}`);
  write(
    dir,
    {
      name,
      version,
      description: `The pixmith-mcp binary for ${target}. Install pixmith-mcp instead; it picks this package for you.`,
      license: base.license,
      repository: base.repository,
      homepage: base.homepage,
      os: [os],
      cpu: [cpu],
      files: ["bin"],
      preferUnplugged: true,
    },
    `# ${name}\n\nThe prebuilt \`pixmith-mcp\` binary for ${target}. Use [\`pixmith-mcp\`](https://www.npmjs.com/package/pixmith-mcp) (\`npx -y pixmith-mcp\`), which installs this package on ${target}.\n`,
  );
  const exe = os === "win32" ? "pixmith-mcp.exe" : "pixmith-mcp";
  fs.mkdirSync(path.join(dir, "bin"));
  fs.copyFileSync(binary, path.join(dir, "bin", exe));
  fs.chmodSync(path.join(dir, "bin", exe), 0o755);
}
console.log(`staged pixmith-mcp ${version} in ${out}: pixmith-mcp${pairs.map((p) => `, mcp-${p.split("=")[0]}`).join("")}`);
