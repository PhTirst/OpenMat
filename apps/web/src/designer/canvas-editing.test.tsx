import { useReducer, useRef, useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
    createDocument,
    createNode,
    historyReducer,
    type UiDocument,
} from "./model";
import { useCanvasEditing } from "./use-canvas-editing";

function initialDocument() {
    const document = createDocument("GestureApp");
    document.root.layout.mode = "absolute";
    document.root.layout.padding = 0;
    document.root.properties = { Width: 600, Height: 400 };
    for (let i = 0; i < 2; i++) {
        const node = createNode("Button", document.root);
        node.layout = {
            ...node.layout,
            x: 40 + i * 160,
            y: 40,
            width: 80,
            height: 32,
        };
        document.root.children.push(node);
    }
    return document;
}
function Harness({
    initial,
    committed,
}: {
    initial: UiDocument;
    committed: (document: UiDocument) => void;
}) {
    const [history, dispatch] = useReducer(historyReducer, {
        past: [],
        present: initial,
        future: [],
    });
    const [ids, select] = useState(initial.root.children.map((n) => n.id));
    const boardRef = useRef<HTMLDivElement>(null);
    const editor = useCanvasEditing({
        document: history.present,
        ids,
        select,
        boardRef,
        scale: 1,
        running: false,
        commit: (document) => {
            committed(document);
            dispatch({ type: "edit", document });
            return true;
        },
        onError: (error) => {
            throw error;
        },
    });
    const shown = editor.visual?.preview ?? history.present;
    return (
        <>
            <div
                ref={boardRef}
                onPointerDownCapture={editor.onPointerDown}
                data-testid="board"
            >
                <div data-ui-id={initial.root.id}>
                    <div className="ui-content">
                        <div className="ui-layout" data-testid="blank">
                            {shown.root.children.map((node, i) => (
                                <div
                                    key={node.id}
                                    data-ui-id={node.id}
                                    data-testid={`node-${i}`}
                                />
                            ))}
                        </div>
                    </div>
                </div>
                <button data-window-resize data-testid="resize" />
            </div>
            <output data-testid="position">
                {shown.root.children.map((n) => n.layout.x).join(",")}
            </output>
            <output data-testid="size">
                {shown.root.properties.Width}×{shown.root.properties.Height}
            </output>
            <output data-testid="selected">{ids.length}</output>
            <output data-testid="history">{history.past.length}</output>
            <button onClick={() => dispatch({ type: "undo" })}>Undo</button>
        </>
    );
}
describe("canvas gesture transactions", () => {
    beforeEach(() => {
        vi.stubGlobal("PointerEvent", MouseEvent);
        vi.spyOn(
            HTMLElement.prototype,
            "getBoundingClientRect",
        ).mockImplementation(function (this: HTMLElement) {
            const index = this.dataset.testid?.startsWith("node-")
                ? Number(this.dataset.testid.slice(5))
                : -1;
            return new DOMRect(
                index < 0 ? 0 : 40 + index * 160,
                index < 0 ? 0 : 40,
                index < 0 ? 600 : 80,
                index < 0 ? 400 : 32,
            );
        });
        vi.spyOn(HTMLElement.prototype, "getClientRects").mockImplementation(
            function (this: HTMLElement) {
                return [this.getBoundingClientRect()] as unknown as DOMRectList;
            },
        );
    });
    afterEach(() => vi.unstubAllGlobals());
    it("previews a group drag and commits once on release, with a single undo", () => {
        const committed = vi.fn();
        render(<Harness initial={initialDocument()} committed={committed} />);
        fireEvent.pointerDown(screen.getByTestId("node-0"), {
            clientX: 50,
            clientY: 50,
            button: 0,
        });
        for (const x of [60, 70, 83])
            fireEvent.pointerMove(window, {
                clientX: x,
                clientY: 50,
                altKey: true,
            });
        expect(committed).not.toHaveBeenCalled();
        expect(screen.getByTestId("position")).toHaveTextContent("73,233");
        fireEvent.pointerUp(window);
        expect(committed).toHaveBeenCalledOnce();
        expect(screen.getByTestId("history")).toHaveTextContent("1");
        fireEvent.click(screen.getByText("Undo"));
        expect(screen.getByTestId("position")).toHaveTextContent("40,200");
    });
    it("cancels an in-progress move and removes its listeners on Escape", () => {
        const committed = vi.fn();
        render(<Harness initial={initialDocument()} committed={committed} />);
        fireEvent.pointerDown(screen.getByTestId("node-0"), {
            clientX: 50,
            clientY: 50,
            button: 0,
        });
        fireEvent.pointerMove(window, { clientX: 80, clientY: 50 });
        fireEvent.keyDown(window, { key: "Escape" });
        fireEvent.pointerUp(window);
        expect(committed).not.toHaveBeenCalled();
        expect(screen.getByTestId("position")).toHaveTextContent("40,200");
    });
    it("marquee-selects intersecting siblings without editing the document", () => {
        const committed = vi.fn();
        render(<Harness initial={initialDocument()} committed={committed} />);
        fireEvent.pointerDown(screen.getByTestId("blank"), {
            clientX: 10,
            clientY: 10,
            button: 0,
        });
        fireEvent.pointerMove(window, { clientX: 150, clientY: 100 });
        fireEvent.pointerUp(window);
        expect(screen.getByTestId("selected")).toHaveTextContent("1");
        expect(committed).not.toHaveBeenCalled();
    });
    it("resizes the preview window as one reversible transaction", () => {
        const committed = vi.fn();
        render(<Harness initial={initialDocument()} committed={committed} />);
        fireEvent.pointerDown(screen.getByTestId("resize"), {
            clientX: 600,
            clientY: 400,
            button: 0,
        });
        fireEvent.pointerMove(window, { clientX: 730, clientY: 480 });
        expect(screen.getByTestId("size")).toHaveTextContent("730×480");
        expect(committed).not.toHaveBeenCalled();
        fireEvent.pointerUp(window);
        expect(committed).toHaveBeenCalledOnce();
        fireEvent.click(screen.getByText("Undo"));
        expect(screen.getByTestId("size")).toHaveTextContent("600×400");
    });
});
