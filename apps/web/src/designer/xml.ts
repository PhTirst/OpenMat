import type { UiValue } from "./catalog";
import {
    DEFAULT_LAYOUT,
    validateDocument,
    type UiDocument,
    type UiLayout,
    type UiNode,
} from "./model";
import { SIZING_DEFAULTS, STRING_LAYOUT_KEYS } from "./layout-schema";

const escape = (value: string): string =>
    value
        .replaceAll("&", "&amp;")
        .replaceAll("<", "&lt;")
        .replaceAll(">", "&gt;")
        .replaceAll('"', "&quot;")
        .replaceAll("'", "&apos;");
export function serializeUi(document: UiDocument): string {
    validateDocument(document);
    let extended = false;
    function node(n: UiNode, depth: number): string {
        if (
            n.type === "ScrollPanel" ||
            Object.keys(n.layout).some((key) =>
                Object.hasOwn(SIZING_DEFAULTS, key),
            )
        )
            extended = true;
        const i = "  ".repeat(depth);
        const content = [
            `${i}<Component id="${escape(n.id)}" type="${escape(n.type)}" name="${escape(n.name)}">`,
            `${i}  <Layout ${Object.entries(n.layout)
                .map(([key, value]) => `${key}="${escape(String(value))}"`)
                .join(" ")} />`,
        ];
        for (const key of Object.keys(n.properties).sort()) {
            const value = n.properties[key]!;
            content.push(
                `${i}  <Property name="${key}" type="${Array.isArray(value) ? "json" : typeof value}">${escape(typeof value === "string" ? value : JSON.stringify(value))}</Property>`,
            );
        }
        for (const key of Object.keys(n.events).sort())
            content.push(
                `${i}  <Event name="${key}" handler="${escape(n.events[key]!)}" />`,
            );
        content.push(
            ...n.children.map((c) => node(c, depth + 1)),
            `${i}</Component>`,
        );
        return content.join("\n");
    }
    const owner =
        document.version !== 1
            ? `class="${escape(document.appClass!)}"`
            : `controller="${escape(document.controller)}"`;
    const tree = node(document.root, 1);
    return `<?xml version="1.0" encoding="UTF-8"?>\n<OpenMatUI version="${document.version}" ${document.kind ? 'kind="component" ' : ""}${owner}${extended ? ' layoutVersion="2"' : ""}>\n${tree}\n</OpenMatUI>\n`;
}
function attributes(element: Element, allowed: string[]): void {
    for (const attribute of element.getAttributeNames())
        if (!allowed.includes(attribute))
            throw new Error(
                `不支持 ${element.tagName} 的属性 ${attribute}，文件未被修改。`,
            );
}
function primitive(value: unknown): value is string | number | boolean {
    return (
        typeof value === "string" ||
        typeof value === "boolean" ||
        (typeof value === "number" && Number.isFinite(value))
    );
}
export function parseUi(source: string): UiDocument {
    if (source.length > 1_000_000) throw new Error("XML 文件超过 1 MB 限制。");
    if (/<!DOCTYPE|<!ENTITY/i.test(source))
        throw new Error("界面 XML 不支持 DTD 或外部实体。");
    const xml = new DOMParser().parseFromString(source, "application/xml");
    if (xml.querySelector("parsererror"))
        throw new Error("XML 语法错误，请检查标签和属性。");
    const root = xml.documentElement;
    const version = Number(root.getAttribute("version"));
    if (
        root.tagName !== "OpenMatUI" ||
        (version !== 1 && version !== 2 && version !== 3)
    )
        throw new Error("需要 OpenMatUI version=1、2 或 3。");
    attributes(
        root,
        version === 3
            ? ["version", "kind", "class", "layoutVersion"]
            : version === 2
              ? ["version", "class", "layoutVersion"]
              : ["version", "controller", "layoutVersion"],
    );
    const layoutVersion = root.getAttribute("layoutVersion");
    if (layoutVersion !== null && layoutVersion !== "2")
        throw new Error("不支持此布局格式版本。");
    let count = 0;
    function node(element: Element, depth: number): UiNode {
        if (++count > 1000 || depth > 32)
            throw new Error("组件数量或嵌套深度超过限制。");
        if (element.tagName !== "Component")
            throw new Error(`无法识别标签 ${element.tagName}。`);
        attributes(element, ["id", "name", "type"]);
        const result: UiNode = {
            id: element.getAttribute("id") ?? "",
            name: element.getAttribute("name") ?? "",
            type: element.getAttribute("type") ?? "",
            properties: {},
            events: {},
            layout: { ...DEFAULT_LAYOUT },
            children: [],
        };
        let layoutSeen = false;
        for (const child of element.children) {
            switch (child.tagName) {
                case "Layout": {
                    if (layoutSeen) throw new Error("Layout 重复。");
                    layoutSeen = true;
                    attributes(child, [
                        ...Object.keys(DEFAULT_LAYOUT),
                        ...(layoutVersion === "2"
                            ? Object.keys(SIZING_DEFAULTS)
                            : []),
                    ]);
                    for (const name of child.getAttributeNames()) {
                        const value = child.getAttribute(name)!;
                        (
                            result.layout as unknown as Record<
                                string,
                                number | string
                            >
                        )[name] = STRING_LAYOUT_KEYS.has(name)
                            ? value
                            : value.trim()
                              ? Number(value)
                              : NaN;
                    }
                    break;
                }
                case "Property": {
                    attributes(child, ["name", "type"]);
                    const name = child.getAttribute("name") ?? "";
                    if (Object.hasOwn(result.properties, name))
                        throw new Error(`属性重复：${name}`);
                    const content = child.textContent ?? "";
                    const type = child.getAttribute("type");
                    let value: unknown;
                    if (type === "string") value = content;
                    else if (type === "number")
                        value = content.trim() ? Number(content) : NaN;
                    else if (type === "boolean") {
                        if (!["true", "false"].includes(content))
                            throw new Error("布尔值必须为 true 或 false。");
                        value = content === "true";
                    } else if (type === "json") {
                        try {
                            value = JSON.parse(content);
                        } catch {
                            throw new Error("数组属性包含无效 JSON。");
                        }
                    } else throw new Error(`不支持属性类型 ${type}。`);
                    if (
                        !primitive(value) &&
                        !(
                            Array.isArray(value) &&
                            (value.every((v) => typeof v === "string") ||
                                value.every(
                                    (row) =>
                                        Array.isArray(row) &&
                                        row.every(primitive),
                                ))
                        )
                    )
                        throw new Error(
                            "属性只支持标量、字符串列表和二维标量表格。",
                        );
                    Object.defineProperty(result.properties, name, {
                        value: value as UiValue,
                        enumerable: true,
                        configurable: true,
                        writable: true,
                    });
                    break;
                }
                case "Event": {
                    attributes(child, ["name", "handler"]);
                    const name = child.getAttribute("name") ?? "";
                    if (Object.hasOwn(result.events, name))
                        throw new Error(`事件重复：${name}`);
                    Object.defineProperty(result.events, name, {
                        value: child.getAttribute("handler") ?? "",
                        enumerable: true,
                        configurable: true,
                        writable: true,
                    });
                    break;
                }
                case "Component":
                    result.children.push(node(child, depth + 1));
                    break;
                default:
                    throw new Error(
                        `不支持标签 ${child.tagName}，文件未被修改。`,
                    );
            }
            if (child.tagName !== "Component" && child.children.length)
                throw new Error(`标签 ${child.tagName} 不能包含子标签。`);
        }
        return result;
    }
    if (root.children.length !== 1)
        throw new Error("一个界面文件需要且只能有一个根窗口。");
    const document: UiDocument = {
        version,
        ...(version !== 1
            ? { appClass: root.getAttribute("class") ?? "" }
            : {}),
        controller: root.getAttribute("controller") ?? "",
        ...(version === 3 && root.getAttribute("kind") === "component"
            ? { kind: "component" as const }
            : {}),
        root: node(root.children[0]!, 0),
    };
    validateDocument(document);
    return document;
}
