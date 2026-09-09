import { useEffect, useState } from "react";
import { CONTENT_LAYOUT } from "./layout";
import type { ComponentSpec } from "./catalog";
import {
    findNode,
    updateNode,
    walk,
    type UiDocument,
    type UiNode,
} from "./model";
import { applySnapshot, buildUiProgram, UiRuntimeSession } from "./runtime";

/** setup/update execute in a disposable native session. Only the resulting
 * presentation is retained; generated children never enter the design history. */
export function useComponentPreviews(
    document: UiDocument,
    catalog: Readonly<Record<string, ComponentSpec>>,
    url: string | undefined,
    sourceName: string,
    enabled: boolean,
    onError: (error: unknown) => void,
): UiNode {
    const [previews, setPreviews] = useState<{
        definition: UiDocument;
        nodes: UiNode[];
    } | null>(null);
    useEffect(() => {
        const components = walk(document.root).filter(
            (node) => catalog[node.type]?.className,
        );
        if (
            !url ||
            !enabled ||
            !components.length ||
            walk(document.root).some((node) => !catalog[node.type])
        )
            return;
        let active = true;
        const definition: UiDocument = {
            version: 1,
            controller: "",
            root: {
                ...document.root,
                type: "Window",
                events: {},
                ...(document.kind ? { properties: {} } : {}),
            },
        };
        const session = UiRuntimeSession.websocket(url, {
            onPayload: (payload) => {
                if (active && payload.kind === "snapshot") {
                    const root = applySnapshot(
                        definition,
                        payload.component,
                    ).root;
                    setPreviews({
                        definition: document,
                        nodes: components.flatMap((node) => {
                            const preview = findNode(root, node.id);
                            return preview ? [preview] : [];
                        }),
                    });
                }
            },
            onDisplay: () => undefined,
            onLog: () => undefined,
            onLost: (error) => {
                if (active) onError(error);
            },
        });
        void (async () => {
            try {
                await session.connect();
                if (active)
                    await session.execute(
                        buildUiProgram(definition, catalog),
                        sourceName,
                    );
            } catch (error) {
                if (active) onError(error);
            } finally {
                await session.dispose();
            }
        })();
        return () => {
            active = false;
            void session.dispose();
        };
    }, [document, catalog, url, sourceName, enabled, onError]);
    if (previews?.definition !== document) return document.root;
    let root = document.root;
    for (const preview of previews.nodes)
        root = updateNode(root, preview.id, (node) => ({
            ...node,
            properties: preview.properties,
            layout: {
                ...preview.layout,
                ...Object.fromEntries(
                    Object.entries(node.layout).filter(
                        ([key]) =>
                            !CONTENT_LAYOUT.has(
                                key as keyof typeof node.layout,
                            ),
                    ),
                ),
            },
            children: preview.children,
        }));
    return root;
}
