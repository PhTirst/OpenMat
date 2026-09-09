import { describe, expect, it } from "vitest";
import {
  deriveLspWebSocketUrl,
  documentPathToUri,
  resolveLspWebSocketUrl,
} from "./url";

describe("OpenMat LSP URL identity", () => {
  it("derives the sibling LSP endpoint without carrying unsafe URL components", () => {
    expect(deriveLspWebSocketUrl("ws://127.0.0.1:49152/kernel")).toBe(
      "ws://127.0.0.1:49152/lsp",
    );
    expect(deriveLspWebSocketUrl("wss://localhost/kernel")).toBe(
      "wss://localhost/lsp",
    );
    expect(deriveLspWebSocketUrl("ws://127.0.0.1:49152/kernel?token=x")).toBeUndefined();
    expect(deriveLspWebSocketUrl("https://127.0.0.1:49152/kernel")).toBeUndefined();
    expect(deriveLspWebSocketUrl("ws://user@127.0.0.1:49152/kernel")).toBeUndefined();
  });

  it("prefers a valid explicit endpoint and rejects an explicit wrong path", () => {
    expect(
      resolveLspWebSocketUrl(
        "ws://127.0.0.1:50000/lsp",
        "ws://127.0.0.1:49152/kernel",
      ),
    ).toBe("ws://127.0.0.1:50000/lsp");
    expect(
      resolveLspWebSocketUrl(
        "ws://127.0.0.1:50000/kernel",
        "ws://127.0.0.1:49152/kernel",
      ),
    ).toBeUndefined();
  });

  it("creates stable file URIs for relative, Windows, and explicit identities", () => {
    expect(documentPathToUri("alpha_demo.m")).toBe(
      "file:///workspace/alpha_demo.m",
    );
    expect(documentPathToUri("C:\\work\\demo.m")).toBe("file:///C:/work/demo.m");
    expect(documentPathToUri("file:///already.m")).toBe("file:///already.m");
  });
});
