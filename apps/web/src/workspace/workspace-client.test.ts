import { describe, expect, it } from "vitest";
// @ts-expect-error Monaco exposes this URI runtime without a declaration file.
import { URI } from "monaco-editor/esm/vs/base/common/uri.js";
import { parseWorkspaceDocumentUri, workspaceDocumentUri } from "./workspace-client";

describe("workspace document URI parsing", () => {
  it.each([
    "C:\\work\\OpenMat\\nested workspace\\信号 %20",
    "/home/user/nested workspace/信号 %20",
    "\\\\server\\share\\workspace",
    "/home/!'()*/literal%2F/root",
  ])("round-trips generated and Monaco canonical URIs for %s", (rootPath) => {
    const path = "目录/nested%20/!'()* source.m";
    const uri = workspaceDocumentUri(path, 41, rootPath);
    const expected = { path, rootPath, rootGeneration: 41 };
    expect(parseWorkspaceDocumentUri(uri)).toEqual(expected);
    expect(parseWorkspaceDocumentUri(URI.parse(uri).toString())).toEqual(expected);
  });

  it("keeps percent-encoded text in actual filenames literal after one decode", () => {
    const uri = workspaceDocumentUri("%2E%2E/%2F.m", 0, "/workspace/%20");
    expect(parseWorkspaceDocumentUri(uri)).toEqual({
      path: "%2E%2E/%2F.m", rootPath: "/workspace/%20", rootGeneration: 0,
    });
  });

  it.each([
    "file:///workspace/helper.m",
    "openmat-designer://root-1/helper.m",
    "openmat-workspace://user@root-1/%252Fworkspace/helper.m",
    "openmat-workspace://root-1:80/%252Fworkspace/helper.m",
    "openmat-workspace://root--1/%252Fworkspace/helper.m",
    "openmat-workspace://root-01/%252Fworkspace/helper.m",
    "openmat-workspace://root-9007199254740992/%252Fworkspace/helper.m",
    "openmat-workspace://root-1/%252Fworkspace/helper.m?query",
    "openmat-workspace://root-1/%252Fworkspace/helper.m#fragment",
    "openmat-workspace://root-1/source/helper.m",
    "openmat-workspace://root-1/%252Fworkspace/",
    "openmat-workspace://root-1/%252Fworkspace/helper.m/",
    "openmat-workspace://root-1/%252Fworkspace/sub//helper.m",
    "openmat-workspace://root-1/%252Fworkspace/sub/../helper.m",
    "openmat-workspace://root-1/%252Fworkspace/./helper.m",
    "openmat-workspace://root-1/%252Fworkspace/%2e%2E/helper.m",
    "openmat-workspace://root-1/%252Fworkspace/sub%2F..%2Fhelper.m",
    "openmat-workspace://root-1/%252Fworkspace/sub\\helper.m",
    "openmat-workspace://root-1/%252Fworkspace/sub%5chelper.m",
    "openmat-workspace://root-1/%252Fworkspace/C%3A/helper.m",
    "openmat-workspace://root-1/%252Fworkspace/C%3Ahelper.m",
    "openmat-workspace://root-1/%252Fworkspace/helper%00.m",
    "openmat-workspace://root-1/%252Fworkspace/helper%.m",
    "openmat-workspace://root-1/%252Fworkspace/helper%FF.m",
    "openmat-workspace://root-1/%252Fworkspace%25/helper.m",
    "openmat-workspace://root-1/%252Fworkspace%2500/helper.m",
  ])("rejects an invalid or non-file workspace URI: %s", (uri) => {
    expect(parseWorkspaceDocumentUri(uri)).toBeNull();
  });
});
