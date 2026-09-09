const WEBSOCKET_PROTOCOLS = new Set(["ws:", "wss:"]);

function parseCleanWebSocketUrl(value: string, expectedPath: string): URL | undefined {
  try {
    const url = new URL(value);
    if (
      !WEBSOCKET_PROTOCOLS.has(url.protocol) ||
      url.username.length > 0 ||
      url.password.length > 0 ||
      url.search.length > 0 ||
      url.hash.length > 0 ||
      url.pathname !== expectedPath
    ) {
      return undefined;
    }
    return url;
  } catch {
    return undefined;
  }
}

export function deriveLspWebSocketUrl(kernelUrl: string | undefined): string | undefined {
  const trimmed = kernelUrl?.trim();
  if (trimmed === undefined || trimmed.length === 0) {
    return undefined;
  }
  const url = parseCleanWebSocketUrl(trimmed, "/kernel");
  if (url === undefined) {
    return undefined;
  }
  url.pathname = "/lsp";
  return url.href;
}

export function resolveLspWebSocketUrl(
  configuredLspUrl: string | undefined,
  kernelUrl: string | undefined,
): string | undefined {
  const trimmed = configuredLspUrl?.trim();
  if (trimmed !== undefined && trimmed.length > 0) {
    return parseCleanWebSocketUrl(trimmed, "/lsp")?.href;
  }
  return deriveLspWebSocketUrl(kernelUrl);
}

export function documentPathToUri(documentPath: string): string {
  const trimmed = documentPath.trim();
  const normalized = trimmed.replaceAll("\\", "/");
  if (/^[a-z]:\//iu.test(normalized)) {
    return encodeURI(`file:///${normalized}`);
  }
  if (/^[a-z][a-z0-9+.-]*:/iu.test(trimmed)) {
    return trimmed;
  }
  const relative = normalized.replace(/^\/+/, "");
  return encodeURI(`file:///workspace/${relative || "untitled.m"}`);
}
