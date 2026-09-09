import {
    BUILTIN_CATALOG,
    type ComponentSpec,
    type MethodSpec,
} from "./catalog";
import { layoutAssignments } from "./layout";
import { matlabPropertyValue, matlabString } from "./runtime";
import {
    IDENTIFIER,
    QUALIFIED_NAME,
    validateDocument,
    walk,
    type UiDocument,
    type UiNode,
} from "./model";

const BEGIN = "% <OpenMat:components>";
const END = "% </OpenMat:components>";
export const BIND_METHOD = "bindDesignerComponents";
export interface ClassSource {
    name: string;
    properties: Set<string>;
    methods: (MethodSpec & { lineNumber: number; static: boolean })[];
    endOffset: number;
    nameOffset: number;
}

/** A lexical block scan for source-preserving edits, not an M interpreter.
 * Strings, comments and indexing end tokens cannot close a class block. */
export function inspectClassSource(
    source: string,
    className: string,
): ClassSource {
    const tokens: { text: string; offset: number }[] = [];
    let i = 0;
    let previous = "";
    const delimiters: string[] = [];
    while (i < source.length) {
        const start = i;
        const c = source[i]!;
        if (c === "%") {
            if (source.startsWith("%{", i)) {
                const end = source.indexOf("%}", i + 2);
                if (end < 0) throw new Error("类文件的块注释未结束。");
                i = end + 2;
            } else {
                while (i < source.length && source[i] !== "\n") i++;
            }
            continue;
        }
        const startsArrayString =
            ["[", "{"].includes(delimiters.at(-1) ?? "") &&
            /\s/.test(source[i - 1] ?? "");
        if (
            c === '"' ||
            (c === "'" && (startsArrayString || !/[\w)\]}]$/.test(previous)))
        ) {
            const quote = c;
            i++;
            let closed = false;
            while (i < source.length) {
                if (source[i++] === quote) {
                    if (source[i] === quote) i++;
                    else {
                        closed = true;
                        break;
                    }
                }
            }
            if (!closed) throw new Error("类文件的字符串未结束。");
            previous = "value";
            continue;
        }
        const word = /^[A-Za-z]\w*/.exec(source.slice(i));
        if (word) {
            i += word[0].length;
            previous = word[0];
        } else {
            i++;
            if (!/[ \t\r]/.test(c)) previous = c;
            if ("([{ ".trim().includes(c)) delimiters.push(c);
            if (")] }".replaceAll(" ", "").includes(c)) delimiters.pop();
        }
        if (!/[ \t\r]/.test(c))
            tokens.push({ text: word?.[0] ?? c, offset: start });
    }
    const stack: {
        kind: string;
        access: MethodSpec["access"];
        static: boolean;
    }[] = [];
    const result: ClassSource = {
        name: className,
        properties: new Set(),
        methods: [],
        endOffset: -1,
        nameOffset: -1,
    };
    let brackets = 0;
    let statement = true;
    let seenClass = false;
    for (const token of tokens) {
        const text = token.text;
        if (text === "\n" || text === ";" || text === ",") {
            if (!brackets) statement = true;
            continue;
        }
        if ("([{ ".trim().includes(text)) brackets++;
        if (")] }".replaceAll(" ", "").includes(text)) brackets--;
        if (text === "end" && brackets === 0 && statement) {
            const block = stack.pop();
            if (block?.kind === "classdef") {
                result.endOffset = token.offset;
                break;
            }
            statement = false;
            continue;
        }
        if (!statement || brackets !== 0) continue;
        statement = false;
        const line = source.slice(token.offset).split(/\r?\n|;/, 1)[0]!;
        if (text === "classdef") {
            const declaration =
                /^classdef\s+(?:\([^)]*\)\s*)?([A-Za-z]\w*)/.exec(line);
            if (seenClass || declaration?.[1] !== className.split(".").at(-1))
                throw new Error(`类文件必须声明 ${className}。`);
            seenClass = true;
            result.nameOffset =
                token.offset + declaration![0].lastIndexOf(declaration![1]!);
        }
        if (
            [
                "classdef",
                "methods",
                "properties",
                "events",
                "enumeration",
                "function",
                "if",
                "for",
                "parfor",
                "while",
                "switch",
                "try",
                "spmd",
            ].includes(text)
        ) {
            const access = /\bAccess\s*=\s*(?:'|")?(private|protected)/.exec(
                line,
            )?.[1] as MethodSpec["access"] | undefined;
            if (text === "function" && stack.at(-1)?.kind === "methods") {
                const declaration =
                    /^function\s+(?:(?:\[[^\]]*\]|[A-Za-z]\w*)\s*=\s*)?([A-Za-z]\w*)/.exec(
                        line,
                    );
                if (declaration)
                    result.methods.push({
                        name: declaration[1]!,
                        access: stack.at(-1)!.access,
                        declaringClass: className,
                        static: stack.at(-1)!.static,
                        lineNumber: source.slice(0, token.offset).split("\n")
                            .length,
                    });
            }
            stack.push({
                kind: text,
                access: access ?? "public",
                static: /\bStatic\b(?!\s*=\s*false)/.test(line),
            });
        } else if (
            stack.at(-1)?.kind === "properties" &&
            IDENTIFIER.test(text)
        ) {
            result.properties.add(text);
        }
    }
    if (!seenClass || result.endOffset < 0)
        throw new Error("需要完整的 classdef 类文件，无法安全插入成员。");
    return result;
}

export function classTemplate(name: string, component = false): string {
    if (!QUALIFIED_NAME.test(name)) throw new Error("应用类名称无效。");
    if (component)
        return `classdef ${name.split(".").at(-1)} < openmat.ui.Panel\n    properties\n        % 在此声明复合组件对外公开的属性。\n    end\n    methods (Access = protected)\n        function update(obj)\n            % 根据公开属性更新内部控件。\n        end\n    end\nend\n`;
    return `classdef ${name.split(".").at(-1)} < openmat.ui.AppBase\n    properties\n        % 在此声明应用状态和可编辑属性。\n    end\n\n    methods (Access = private)\n        function onStartup(app, source, event)\n            % 所有命名控件已绑定到 app 的成员属性。\n        end\n    end\nend\n`;
}

/** Rename only the class declaration and constructor header; bodies are owned by the user. */
export function renameClassSource(
    source: string,
    from: string,
    to: string,
): string {
    if (!QUALIFIED_NAME.test(to)) throw new Error("类名称无效。");
    const info = inspectClassSource(source, from);
    const oldName = from.split(".").at(-1)!;
    const newName = to.split(".").at(-1)!;
    const edits = [
        { offset: info.nameOffset, length: oldName.length, text: newName },
    ];
    const constructor = info.methods.find((m) => m.name === oldName);
    if (constructor) {
        const lines = source.split("\n");
        const line = lines[constructor.lineNumber - 1]!;
        const match = new RegExp(`\\b${oldName}(?=\\s*\\()`).exec(line);
        if (match)
            edits.push({
                offset:
                    lines.slice(0, constructor.lineNumber - 1).join("\n")
                        .length +
                    1 +
                    match.index,
                length: oldName.length,
                text: newName,
            });
    }
    for (const edit of edits.sort((a, b) => b.offset - a.offset))
        source =
            source.slice(0, edit.offset) +
            edit.text +
            source.slice(edit.offset + edit.length);
    return source;
}

export function ensureClassMethod(
    source: string,
    className: string,
    method: string,
    access = "private",
): string {
    if (
        !IDENTIFIER.test(method) ||
        [
            BIND_METHOD,
            "buildDesignerComponents",
            "applyDesignerDefaults",
        ].includes(method)
    )
        throw new Error("成员方法名称无效或被设计器保留。");
    const info = inspectClassSource(source, className);
    const existing = info.methods.find((m) => m.name === method);
    if (existing?.static)
        throw new Error("事件需要实例成员方法，不能绑定静态方法。");
    if (existing) return source;
    const addition = `    methods (Access = ${access})\n        function ${method}(obj, source, event)\n            % 在此处理事件。obj 是此类的当前实例。\n        end\n    end\n`;
    return (
        source.slice(0, info.endOffset) +
        addition +
        source.slice(info.endOffset)
    );
}

export function syncAppClass(
    source: string,
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>> = BUILTIN_CATALOG,
): string {
    if (document.version === 1) return source;
    validateDocument(document);
    const hasBegin = source.includes(BEGIN),
        hasEnd = source.includes(END);
    if (hasBegin !== hasEnd)
        throw new Error("设计器成员标记不完整，请恢复完整标记后保存。");
    const region =
        /^ *% <OpenMat:components>\r?\n[\s\S]*?^ *% <\/OpenMat:components>\r?\n?/gm;
    const clean = source.replace(region, "");
    if (clean.includes(BEGIN) || clean.includes(END))
        throw new Error("设计器成员标记格式无效。");
    const info = inspectClassSource(clean, document.appClass!);
    if (
        info.methods.some((m) =>
            [
                BIND_METHOD,
                "buildDesignerComponents",
                "applyDesignerDefaults",
            ].includes(m.name),
        )
    )
        throw new Error(`${BIND_METHOD} 是设计器保留的方法。`);
    const nodes = walk(document.root);
    const fields = nodes.filter((n) => !info.properties.has(n.name));
    const lines = ["    " + BEGIN];
    const component = document.kind === "component";
    const shortName = document.appClass!.split(".").at(-1)!;
    if (
        component &&
        info.methods.some((m) => m.name === shortName) &&
        !/\.applyDesignerDefaults\s*\(/.test(clean)
    )
        throw new Error(
            "自定义组件构造函数需要调用 obj.applyDesignerDefaults()，以便先应用设计默认值。",
        );
    if (fields.length)
        lines.push(
            "    properties (SetAccess = private)",
            ...fields.map((n) => `        ${n.name}`),
            "    end",
        );
    if (component) {
        if (!info.methods.some((m) => m.name === shortName))
            lines.push(
                "    methods",
                `        function app = ${shortName}()`,
                "            app.applyDesignerDefaults();",
                "        end",
                "    end",
            );
        lines.push(
            "    methods (Access = protected)",
            "        function applyDesignerDefaults(app)",
        );
        const rootSpec =
            catalog[document.appClass!] ?? catalog[document.root.type]!;
        for (const property of rootSpec.properties) {
            const value = document.root.properties[property.name];
            if (value !== undefined && !property.readonly)
                lines.push(
                    `            app.${property.name} = ${matlabPropertyValue(property, value)};`,
                );
        }
        lines.push(
            ...layoutAssignments("app", document.root.layout).map(
                (line) => `            ${line}`,
            ),
            "        end",
            "        function buildDesignerComponents(app)",
            "            components = struct();",
            `            components.${document.root.name} = app;`,
        );
        for (const node of nodes.slice(1)) {
            const spec = catalog[node.type];
            if (!spec) throw new Error(`请先注册组件类 ${node.type}。`);
            const target = `components.${node.name}`;
            lines.push(
                `            ${target} = ${spec.className ?? `openmat.ui.${node.type}`}();`,
                `            ${target}.Name = ${matlabString(node.name)};`,
            );
            for (const property of spec.properties) {
                const value = node.properties[property.name];
                if (
                    !property.readonly &&
                    (value !== undefined || !spec.className)
                )
                    lines.push(
                        `            ${target}.${property.name} = ${matlabPropertyValue(property, value ?? property.default)};`,
                    );
            }
            lines.push(
                ...layoutAssignments(target, node.layout, !spec.composite).map(
                    (line) => `            ${line}`,
                ),
            );
        }
        for (const node of nodes)
            for (const child of node.children)
                lines.push(
                    `            components.${node.name}.add(components.${child.name});`,
                );
    } else
        lines.push(
            "    methods",
            `        function ${BIND_METHOD}(app, components)`,
        );
    for (const node of nodes)
        lines.push(`            app.${node.name} = components.${node.name};`);
    for (const node of nodes)
        for (const [event, binding] of Object.entries(node.events)) {
            const [target, method] = binding.split(".");
            const receiver =
                target === "app" || target === document.root.name
                    ? "app"
                    : `app.${target === "self" ? node.name : target}`;
            lines.push(
                `            app.listen(app.${node.name}, '${event}', @(source, event) ${receiver}.${method}(source, event));`,
            );
        }
    lines.push("        end", "    end", "    " + END, "");
    return (
        clean.slice(0, info.endOffset) +
        lines.join("\n") +
        clean.slice(info.endOffset)
    );
}

export function methodTarget(
    binding: string,
    node: UiNode,
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>>,
) {
    const [receiver, method, extra] = binding.split(".");
    if (!receiver || !method || extra || !IDENTIFIER.test(method))
        throw new Error("需要 对象.成员方法，例如 app.onRun。");
    const target =
        receiver === "app"
            ? document.root
            : receiver === "self"
              ? node
              : walk(document.root).find((n) => n.name === receiver);
    if (!target) throw new Error(`未找到事件接收对象 ${receiver}。`);
    const isApp = target.id === document.root.id;
    const className = isApp
        ? document.appClass
        : catalog[target.type]?.className;
    if (!className)
        throw new Error("请将事件绑定到应用类或自定义组件类的方法。");
    const metadata = catalog[className]?.methods?.find(
        (m) => m.name === method,
    );
    if (
        metadata &&
        metadata.access !== "public" &&
        (!isApp || metadata.declaringClass !== document.appClass)
    )
        throw new Error("其他组件及继承的私有方法不能从应用类直接绑定。");
    return {
        className: metadata?.declaringClass ?? className,
        methodName: method,
        isApp,
    };
}

export function methodOptions(
    source: string,
    document: UiDocument,
    node: UiNode,
    catalog: Readonly<Record<string, ComponentSpec>>,
) {
    if (document.version === 1) return [];
    let own: MethodSpec[] = [];
    try {
        own = inspectClassSource(source, document.appClass!).methods.filter(
            (m) => !m.static,
        );
    } catch {
        /* Editing may be incomplete. */
    }
    const options: { value: string; className: string }[] = [];
    for (const target of walk(document.root)) {
        const isApp = target.id === document.root.id;
        const spec = catalog[isApp ? document.appClass! : target.type];
        const methods = isApp
            ? [
                  ...own,
                  ...(spec?.methods ?? []).filter((m) => m.access === "public"),
              ]
            : (spec?.methods ?? []).filter((m) => m.access === "public");
        for (const method of methods) {
            if (
                [
                    BIND_METHOD,
                    "setup",
                    "update",
                    "buildDesignerComponents",
                    "applyDesignerDefaults",
                    document.appClass!.split(".").at(-1),
                ].includes(method.name)
            )
                continue;
            const value = `${isApp ? "app" : target.id === node.id ? "self" : target.name}.${method.name}`;
            if (!options.some((o) => o.value === value))
                options.push({ value, className: method.declaringClass });
        }
    }
    return options;
}
