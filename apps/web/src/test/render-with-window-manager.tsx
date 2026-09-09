import { render, type RenderResult } from "@testing-library/react";
import type { ReactNode } from "react";

import { createOpenMatAppRegistry } from "../windowing/built-in-apps";
import {
  OpenMatWindowLayer,
  OpenMatWindowManagerProvider,
} from "../windowing/WindowManager";

export function renderWithWindowManager(children: ReactNode): RenderResult {
  const registry = createOpenMatAppRegistry();
  const wrap = (content: ReactNode) => (
    <OpenMatWindowManagerProvider registry={registry}>
      {content}
      <OpenMatWindowLayer />
    </OpenMatWindowManagerProvider>
  );
  const result = render(wrap(children));
  const baseRerender = result.rerender;
  return {
    ...result,
    rerender: (next) => baseRerender(wrap(next)),
  };
}
