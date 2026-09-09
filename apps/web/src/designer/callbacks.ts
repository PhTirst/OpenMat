import { QUALIFIED_NAME } from "./model";

export function callbackPath(handler: string, uiPath: string): string {
    if (!QUALIFIED_NAME.test(handler)) throw new Error("回调函数名称无效。");
    const normalized = uiPath.replaceAll("\\", "/");
    let folder = normalized.slice(0, normalized.lastIndexOf("/") + 1);
    const segments = handler.split(".");
    const packageFolder = segments
        .slice(0, -1)
        .map((name) => `+${name}/`)
        .join("");
    if (packageFolder && folder.endsWith(packageFolder))
        folder = folder.slice(0, -packageFolder.length);
    return (
        folder +
        segments
            .map((name, index) =>
                index === segments.length - 1 ? `${name}.m` : `+${name}`,
            )
            .join("/")
    );
}

export function renamedComponentPath(
    from: string,
    to: string,
    uiPath: string,
): string {
    const normalized = uiPath.replaceAll("\\", "/");
    if (normalized.split("/").at(-1) !== `${from.split(".").at(-1)}.omui`)
        return uiPath;
    const oldRelativeSource = from
        .split(".")
        .map((name, index, names) =>
            index === names.length - 1 ? `${name}.m` : `+${name}`,
        )
        .join("/");
    const folder = callbackPath(from, uiPath).slice(
        0,
        -oldRelativeSource.length,
    );
    return callbackPath(to, `${folder}component.omui`).replace(/\.m$/, ".omui");
}

export function callbackTemplate(handler: string): string {
    if (!QUALIFIED_NAME.test(handler)) throw new Error("回调函数名称无效。");
    return `function ${handler.split(".").at(-1)}(app, event)\n% 在这里处理组件事件，使用 app.组件名称 访问控件。\n\nend\n`;
}

/** Prefer the component's dispatch branch in a shared application callback. */
export function callbackLine(
    source: string,
    handler: string,
    component?: string,
): number {
    const lines = source.split(/\r?\n/);
    const branch = component
        ? lines.findIndex(
              (line) =>
                  !line.trimStart().startsWith("%") &&
                  (line.includes(`'${component}'`) ||
                      line.includes(`"${component}"`)),
          )
        : -1;
    if (branch >= 0) return branch + 1;
    const name = handler.split(".").at(-1)!;
    const declaration = lines.findIndex(
        (line) =>
            /^\s*function\b/.test(line) &&
            new RegExp(`\\b${name}\\s*\\(`).test(line),
    );
    return Math.max(1, declaration + 1);
}
