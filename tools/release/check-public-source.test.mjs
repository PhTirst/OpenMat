import assert from "node:assert/strict";
import test from "node:test";
import { contentIssues, pathIssue } from "./check-public-source.mjs";

test("rejects dependency sources and generated binaries at nested paths", () => {
  for (const name of ["target/debug/x.rs", "plugins/excel/dist/third-party/x.rs", "apps/web/node_modules/react/index.js", "vendor/openblas/a.c", "apps/desktop/src-tauri/resources/openblas/x.dll", "output/report.json", "docs/handoff/note.md", "archive.tar.gz", "save1.mat"]) assert.ok(pathIssue(name), name);
});
test("retains providers, locks, fixtures, icons, and upstream license texts", () => {
  for (const name of ["crates/openmat-openblas/src/lib.rs", "Cargo.lock", "apps/web/pnpm-lock.yaml", "tests/conformance/reference/matlab-r2022b/test.json", "tools/release/licenses/faer-MIT.txt", "apps/desktop/src-tauri/icons/icon.ico", ".env.example"]) assert.equal(pathIssue(name), null, name);
});
test("rejects private config while allowing documented templates", () => {
  assert.ok(pathIssue(".env.production"));
  assert.ok(pathIssue("apps/web/.env.local"));
  assert.equal(pathIssue("apps/web/.env.local.example"), null);
});
test("reports credential rule and line without disclosing the matched value", () => {
  const token = "ghp_" + "a".repeat(36);
  const issues = contentIssues("sample.txt", Buffer.from(`example\n${token}\n`));
  assert.equal(issues.length, 1);
  assert.match(issues[0], /credential.*line 2/);
  assert.ok(!issues[0].includes(token));
});
test("detects keys and personal paths without rejecting loopback fixtures", () => {
  const key = ["-----BEGIN", "OPENSSH PRIVATE KEY-----"].join(" ");
  assert.ok(contentIssues("sample.txt", Buffer.from(key)).length);
  const personal = ["C:", "Users", "alice", "project"].join("\\");
  assert.ok(contentIssues("sample.txt", Buffer.from(personal)).length);
  assert.deepEqual(contentIssues("sample.txt", Buffer.from("http://127.0.0.1:5173")), []);
  assert.deepEqual(contentIssues("sample.txt", Buffer.from("file:///openmat-dev-lsp-smoke.m")), []);
});
test("requires package license without requiring one on virtual workspaces", () => {
  assert.deepEqual(contentIssues("Cargo.toml", Buffer.from("[workspace]\nmembers = []\n")), []);
  assert.equal(contentIssues("Cargo.toml", Buffer.from('[package]\nname = "sample"\n[dependencies]\n')).length, 1);
  assert.deepEqual(contentIssues("Cargo.toml", Buffer.from('[package]\nlicense = "AGPL-3.0-only"\n[dependencies]\n')), []);
  assert.equal(contentIssues("package.json", Buffer.from('{"private":true}')).length, 1);
  assert.deepEqual(contentIssues("package.json", Buffer.from('{"license":"AGPL-3.0-only"}')), []);
});
