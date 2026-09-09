import { spawnSync } from "node:child_process";
import { lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const generatedPart = /^(?:target|node_modules|dist|vendor|third[_-]party|\.build|\.openmat(?:-.*)?|\.pnpm-store|\.VSCodeCounter|\.playwright-cli|\.codex|\.agents)$/i;
const privateTop = /^(?:tmp|output|public-release|local-tests|my_test|docs\/handoff)(?:\/|$)/i;
const binarySuffix = /\.(?:dll|exe|msi|lib|a|so(?:\.\d+)*|dylib|wasm|zip|7z|tar|gz|crate|pdb|mat|ombc|log|tsbuildinfo|pem|key|p12|pfx)$/i;

export function pathIssue(name) {
  const parts = name.split("/");
  if (parts.some((part) => part === ".." || generatedPart.test(part)) || privateTop.test(name)) return "generated, private, or third-party source path";
  if (/^apps\/desktop\/src-tauri\/resources\/(?:openblas|licenses)(?:\/|$)/i.test(name)) return "generated installer resource";
  if (/^EULA(?:[_.]|$)/i.test(name) || /(?:^|\/)\.env(?:\.|$)/.test(name) && !/\.example$/.test(name)) return "private configuration or obsolete license file";
  return binarySuffix.test(name) ? "binary, archive, key, or generated file" : null;
}

const contentRules = [
  ["private key", /-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----/],
  ["GitHub credential", /\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{40,})\b/],
  ["AWS credential", /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/],
  ["service credential", /\b(?:xox[baprs]-[A-Za-z0-9-]{20,}|sk-(?:proj-)?[A-Za-z0-9_-]{40,})\b/],
  ["personal Windows profile path", /[A-Za-z]:[\\/]+Users[\\/]+(?!Public\b|Default\b)[^\s<>"']+/i],
  ["private original checkout path", /(?<![A-Za-z])E:[\\/]+OpenMat\b/i],
  ["obsolete proprietary declaration", /(?:OpenMat|Project code) is (?:currently )?proprietary|no AGPL license grant (?:is made|applies)/i],
];

export function contentIssues(name, bytes) {
  if (bytes.includes(0)) return [];
  const text = bytes.toString("utf8");
  const issues = [];
  for (const [rule, pattern] of contentRules) {
    const match = pattern.exec(text);
    if (match) issues.push(`${rule} (line ${text.slice(0, match.index).split("\n").length})`);
  }
  if (name.endsWith("Cargo.toml")) {
    const pkg = /^\[package\]\s*\r?\n([\s\S]*?)(?=^\[|(?![\s\S]))/m.exec(text);
    if (pkg && !/^license\s*=\s*"AGPL-3\.0-only"\s*$/m.test(pkg[1])) issues.push("first-party Cargo package needs AGPL-3.0-only metadata");
  }
  if (name.endsWith("package.json")) {
    try {
      if (JSON.parse(text).license !== "AGPL-3.0-only") issues.push("first-party npm package needs AGPL-3.0-only metadata");
    } catch { issues.push("invalid package.json"); }
  }
  return issues;
}

function git(args, input) {
  const result = spawnSync("git", ["-C", root, ...args], { input, maxBuffer: 128 * 1024 * 1024 });
  if (result.error || result.status !== 0) throw new Error(`Git ${args[0]} failed; use --snapshot for a clean export without Git metadata.`);
  return result.stdout;
}

function snapshotFiles(directory = root, prefix = "") {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name === ".git") continue;
    const name = `${prefix}${entry.name}`;
    // Reject forbidden directories without reading their potentially private contents.
    if (entry.isDirectory() && !pathIssue(name)) files.push(...snapshotFiles(path.join(directory, entry.name), `${name}/`));
    else files.push(name);
  }
  return files;
}

function stagedEntries() {
  const entries = git(["ls-files", "--stage", "-z"]).toString("utf8").split("\0").filter(Boolean).map((line) => {
    const match = /^(\d+) ([a-f0-9]+) (\d)\t([\s\S]+)$/.exec(line);
    if (!match || match[3] !== "0") throw new Error("Resolve unmerged index entries before checking public source.");
    return { mode: match[1], oid: match[2], name: match[4] };
  });
  if (!entries.length) return [];
  // Object IDs avoid interpreting file names as Git revision expressions.
  const response = git(["cat-file", "--batch"], entries.map((entry) => entry.oid).join("\n") + "\n");
  let offset = 0;
  return entries.map((entry) => {
    const end = response.indexOf(10, offset);
    const header = response.subarray(offset, end).toString("ascii");
    const match = /^([a-f0-9]+) blob (\d+)$/.exec(header);
    if (!match || match[1] !== entry.oid) throw new Error("Index object could not be inspected as a source file.");
    const size = Number(match[2]);
    offset = end + 1;
    const bytes = response.subarray(offset, offset + size);
    offset += size + 1;
    return { ...entry, bytes };
  });
}

export function main(args) {
  if (args.some((arg) => !["--snapshot", "--staged"].includes(arg)) || args.length > 1) throw new Error("Usage: node tools/release/check-public-source.mjs [--snapshot | --staged]");
  if (!args.includes("--snapshot")) {
    const top = git(["rev-parse", "--show-toplevel"]).toString("utf8").trim();
    if (realpathSync(top) !== realpathSync(root)) throw new Error("Refusing to inspect an enclosing repository; use --snapshot for this export.");
  }
  const entries = args.includes("--staged") ? stagedEntries() :
    (args.includes("--snapshot") ? snapshotFiles() : git(["ls-files", "-z"]).toString("utf8").split("\0").filter(Boolean)).map((name) => ({ name }));
  const findings = [];
  const contents = new Map();
  for (const entry of entries) {
    const issue = pathIssue(entry.name);
    if (issue) { findings.push([entry.name, issue]); continue; }
    if (entry.mode && !/^100(?:644|755)$/.test(entry.mode)) { findings.push([entry.name, "submodules and symbolic links are not source snapshots"]); continue; }
    if (!entry.bytes && !lstatSync(path.join(root, entry.name)).isFile()) { findings.push([entry.name, "expected a regular source file"]); continue; }
    const bytes = entry.bytes ?? readFileSync(path.join(root, entry.name));
    contents.set(entry.name, bytes);
    for (const detail of contentIssues(entry.name, bytes)) findings.push([entry.name, detail]);
  }
  for (const name of ["LICENSE", "README.md", "CONTRIBUTING.md", ".gitignore", "Cargo.lock", "apps/desktop/src-tauri/Cargo.lock", "apps/web/pnpm-lock.yaml"]) {
    if (!contents.has(name)) findings.push([name, "required public source file is missing"]);
  }
  const license = contents.get("LICENSE")?.toString("utf8") ?? "";
  if (!license.includes("GNU AFFERO GENERAL PUBLIC LICENSE") || !license.includes("END OF TERMS AND CONDITIONS")) findings.push(["LICENSE", "expected complete AGPLv3 text"]);
  for (const [name, issue] of findings) console.error(`${JSON.stringify(name)}: ${issue}`);
  console.log(`Public-source check: ${entries.length} entries, ${findings.length} findings (${args[0] ?? "tracked working files"}).`);
  return findings.length ? 1 : 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { process.exitCode = main(process.argv.slice(2)); }
  catch (error) { console.error(error.message); process.exitCode = 1; }
}
