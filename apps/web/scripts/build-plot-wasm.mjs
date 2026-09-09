import { spawnSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const scriptDirectory = path.dirname(fileURLToPath(import.meta.url));
const repositoryRoot = path.resolve(scriptDirectory, "../../..");
const release = process.argv.includes("--release");
const profile = release ? "release" : "debug";
const configuredTarget = process.env.CARGO_TARGET_DIR;
const targetDirectory = configuredTarget
  ? path.resolve(repositoryRoot, configuredTarget)
  : path.join(repositoryRoot, "target");
const outputDirectory = path.join(repositoryRoot, "apps", "web", "public", "wasm");

function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: repositoryRoot,
    env: process.env,
    stdio: "inherit",
  });
  if (result.error !== undefined) {
    if (command === "wasm-bindgen") {
      console.error(
        "wasm-bindgen CLI is required; install the lock-compatible tool with " +
          "`cargo install wasm-bindgen-cli --version 0.2.127 --locked`.",
      );
    } else {
      console.error(`Unable to launch ${command}: ${result.error.message}`);
    }
    process.exit(1);
  }
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

const cargoArguments = [
  "build",
  "--locked",
  "--package",
  "openmat-plot-web",
  "--target",
  "wasm32-unknown-unknown",
];
if (release) {
  cargoArguments.push("--release");
}

run("cargo", cargoArguments);
mkdirSync(outputDirectory, { recursive: true });
run("wasm-bindgen", [
  path.join(
    targetDirectory,
    "wasm32-unknown-unknown",
    profile,
    "openmat_plot_web.wasm",
  ),
  "--out-dir",
  outputDirectory,
  "--out-name",
  "openmat_plot_web",
  "--target",
  "web",
  "--no-typescript",
]);
