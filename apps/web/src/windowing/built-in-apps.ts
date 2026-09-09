import {
  graphicsFigureAppDefinition,
  invalidFigureAppDefinition,
  legacyFigureAppDefinition,
} from "../components/FigureWindow";
import { variableEditorAppDefinition } from "../components/VariableEditorWindow";
import type { OpenMatAppDefinition } from "./app-registry";
import { OpenMatAppRegistry } from "./app-registry";

function registerIfMissing<Payload>(
  registry: OpenMatAppRegistry,
  definition: OpenMatAppDefinition<Payload>,
): void {
  if (!registry.has(definition.id)) {
    registry.register(definition);
  }
}

export function registerBuiltInOpenMatApps(registry: OpenMatAppRegistry): void {
  registerIfMissing(registry, graphicsFigureAppDefinition);
  registerIfMissing(registry, legacyFigureAppDefinition);
  registerIfMissing(registry, invalidFigureAppDefinition);
  registerIfMissing(registry, variableEditorAppDefinition);
}

export function createOpenMatAppRegistry(): OpenMatAppRegistry {
  const registry = new OpenMatAppRegistry();
  registerBuiltInOpenMatApps(registry);
  return registry;
}
