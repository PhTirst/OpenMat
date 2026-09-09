import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LibraryPanels } from "./LibraryPanels";

const mount = () =>
    render(
        <LibraryPanels palette={<div>Palette</div>} tree={<div>Tree</div>} />,
    );
describe("library and object tree divider", () => {
    beforeEach(() => {
        localStorage.clear();
        vi.stubGlobal("PointerEvent", MouseEvent);
    });
    afterEach(() => vi.unstubAllGlobals());
    it("resizes on pointer input, persists on release and restores the preference after reopening", () => {
        const view = mount();
        const divider = screen.getByRole("separator", {
            name: "组件库与对象树高度",
        });
        const before = Number(divider.getAttribute("aria-valuenow"));
        fireEvent.pointerDown(divider, { button: 0, clientY: 300 });
        fireEvent.pointerMove(window, { clientY: 350 });
        expect(Number(divider.getAttribute("aria-valuenow"))).toBeGreaterThan(
            before,
        );
        expect(localStorage.getItem("openmat.designer.library-split.v1")).toBe(
            "60",
        );
        fireEvent.pointerUp(window);
        const after = divider.getAttribute("aria-valuenow");
        view.unmount();
        mount();
        expect(screen.getByRole("separator")).toHaveAttribute(
            "aria-valuenow",
            after,
        );
    });
    it("cancels a drag, supports keyboard adjustment and resets by double-click", () => {
        mount();
        const divider = screen.getByRole("separator");
        fireEvent.pointerDown(divider, { button: 0, clientY: 300 });
        fireEvent.pointerMove(window, { clientY: 0 });
        fireEvent.keyDown(window, { key: "Escape" });
        fireEvent.pointerUp(window);
        expect(divider).toHaveAttribute("aria-valuenow", "60");
        fireEvent.keyDown(divider, { key: "ArrowUp" });
        expect(divider).toHaveAttribute("aria-valuenow", "55");
        fireEvent.keyDown(divider, { key: "End" });
        expect(Number(divider.getAttribute("aria-valuenow"))).toBeLessThan(90);
        fireEvent.doubleClick(divider);
        expect(divider).toHaveAttribute("aria-valuenow", "60");
    });
});
