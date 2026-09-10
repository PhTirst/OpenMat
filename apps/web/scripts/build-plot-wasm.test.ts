import { spawnSync } from "node:child_process";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "build-plot-wasm.mjs");
const target = path.join(tmpdir(), "openmat-wasm-build-test");
// Exercise the real CLI without compiling Rust or touching generated artifacts.
const interceptCommands = `
  import childProcess from "node:child_process";
  import fs from "node:fs";
  import { syncBuiltinESMExports } from "node:module";
  childProcess.spawnSync = (command, args) => {
    console.log(JSON.stringify({ command, args }));
    return { status: command === "cargo" ? Number(process.env.OPENMAT_TEST_CARGO_EXIT) : 0 };
  };
  fs.mkdirSync = () => {};
  syncBuiltinESMExports();
`;

function invoke(args: string[], cargoExit = 0) {
  const result = spawnSync(
    process.execPath,
    ["--import", `data:text/javascript,${encodeURIComponent(interceptCommands)}`, script, ...args],
    {
      encoding: "utf8",
      env: {
        ...process.env,
        CARGO_TARGET_DIR: target,
        OPENMAT_TEST_CARGO_EXIT: String(cargoExit),
      },
      timeout: 10_000,
    },
  );
  expect(result.error).toBeUndefined();
  const commands = result.stdout
    .split(/\r?\n/)
    .filter((line) => line.startsWith("{"))
    .map((line) => JSON.parse(line) as { command: string; args: string[] });
  return { ...result, commands };
}

describe("Plot Engine WASM build", () => {
  it.each([
    { args: [], profile: "release" },
    { args: ["--release"], profile: "release" },
    { args: ["--debug"], profile: "debug" },
  ])("selects $profile for $args", ({ args, profile }) => {
    const result = invoke(args);
    expect(result.status).toBe(0);
    expect(result.commands.map(({ command }) => command)).toEqual(["cargo", "wasm-bindgen"]);
    expect(result.commands[0].args.includes("--release")).toBe(profile === "release");
    expect(result.commands[1].args[0]).toBe(
      path.join(target, "wasm32-unknown-unknown", profile, "openmat_plot_web.wasm"),
    );
  });

  it("rejects conflicting profiles before launching build tools", () => {
    const result = invoke(["--release", "--debug"]);
    expect(result.status).toBe(1);
    expect(result.commands).toEqual([]);
  });

  it("stops before binding an old artifact when compilation fails", () => {
    const result = invoke([], 7);
    expect(result.status).toBe(7);
    expect(result.commands.map(({ command }) => command)).toEqual(["cargo"]);
  });
});
