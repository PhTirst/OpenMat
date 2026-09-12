import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import SlxPreview from "./SlxPreview";
import type { SlxImport } from "./client";

describe("SLX inspection", () => {
    it("renders serialized block types and subsystem identities, and exposes original parameters", () => {
        const imported: SlxImport = {
            runnable: false,
            issues: [
                {
                    code: "unsupported_block",
                    message: "Unsupported TransferFcn",
                    block: "2",
                },
            ],
            document: {
                name: "unsupported.slx",
                matlabRelease: "R2022b",
                systems: [
                    {
                        parentBlock: null,
                        blocks: [
                            {
                                sid: "1",
                                name: "Plant",
                                blockType: "SubSystem",
                                properties: {},
                                source: { part: "simulink/blockdiagram.xml" },
                            },
                        ],
                        lines: [],
                    },
                    {
                        parentBlock: "1",
                        blocks: [
                            {
                                sid: "2",
                                name: "Response",
                                blockType: "TransferFcn",
                                properties: {
                                    Numerator: "[1]",
                                    Position: "[20 30 50 60]",
                                },
                                source: {
                                    part: "simulink/systems/system_1.xml",
                                },
                            },
                        ],
                        lines: [],
                    },
                ],
            },
        };
        render(<SlxPreview result={imported} onBack={vi.fn()} />);
        expect(
            screen.getByRole("button", { name: "Plant (SubSystem)" }),
        ).toBeInTheDocument();
        fireEvent.change(screen.getByRole("combobox", { name: "SLX 系统" }), {
            target: { value: "1" },
        });
        expect(
            screen.getByRole("option", { name: "子系统 1 (1)" }),
        ).toBeInTheDocument();
        fireEvent.click(
            screen.getByRole("button", { name: "Response (TransferFcn)" }),
        );
        expect(screen.getByText("Numerator")).toBeInTheDocument();
        expect(screen.getByText("[1]")).toBeInTheDocument();
        expect(
            screen.getByText("simulink/systems/system_1.xml"),
        ).toBeInTheDocument();
    });
});
