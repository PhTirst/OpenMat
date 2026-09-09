import type {
  Capabilities,
  KernelEvent,
  KernelProtocol,
  MatrixPreview,
} from "../protocol/kernel-v2";
import type {
  DisplayEventData,
  KernelStatus,
  StreamKind,
  VariableSummary,
} from "../protocol/kernel-v0";

export type ConnectionState =
  | "disconnected"
  | "connecting"
  | "connected"
  | "error";

export interface CommandWindowEntry {
  readonly id: string;
  readonly channel: StreamKind | "input" | "system";
  readonly text: string;
}

export interface InspectionState {
  readonly name: string;
  readonly preview: MatrixPreview;
}

export interface IdeState {
  readonly connection: ConnectionState;
  readonly kernelStatus: KernelStatus;
  readonly negotiatedProtocol: KernelProtocol | null;
  readonly capabilities: Capabilities | null;
  readonly activeRequestId: string | null;
  readonly commandWindowEntries: readonly CommandWindowEntry[];
  readonly figures: readonly DisplayEventData[];
  readonly workspace: readonly VariableSummary[];
  readonly inspection: InspectionState | null;
  readonly inspectingName: string | null;
  readonly error: string | null;
}

export type IdeAction =
  | { readonly type: "connectionStarted" }
  | {
      readonly type: "connectionReady";
      readonly protocol: KernelProtocol;
      readonly capabilities: Capabilities;
    }
  | { readonly type: "connectionFailed"; readonly message: string }
  | {
      readonly type: "executionStarted";
      readonly requestId: string;
      readonly command?: string;
    }
  | { readonly type: "executionFinished"; readonly requestId: string }
  | { readonly type: "executionFailed"; readonly message: string }
  | { readonly type: "workspaceLoaded"; readonly variables: readonly VariableSummary[] }
  | { readonly type: "inspectionStarted"; readonly name: string }
  | {
      readonly type: "inspectionLoaded";
      readonly name: string;
      readonly preview: MatrixPreview;
    }
  | { readonly type: "operationFailed"; readonly message: string }
  | { readonly type: "systemMessage"; readonly message: string }
  | { readonly type: "commandWindowCleared" }
  | { readonly type: "figureClosed"; readonly index: number }
  | { readonly type: "eventReceived"; readonly event: KernelEvent };

export const initialIdeState: IdeState = {
  connection: "disconnected",
  kernelStatus: "starting",
  negotiatedProtocol: null,
  capabilities: null,
  activeRequestId: null,
  commandWindowEntries: [],
  figures: [],
  workspace: [],
  inspection: null,
  inspectingName: null,
  error: null,
};

const COMMAND_WINDOW_CLEAR_MIME =
  "application/vnd.openmat.command-window-clear+json";

function applyWorkspaceDelta(
  current: readonly VariableSummary[],
  delta: Extract<
    KernelEvent,
    { readonly event: { readonly type: "workspaceDelta" } }
  >["event"]["data"],
): readonly VariableSummary[] {
  const byName = new Map(current.map((variable) => [variable.name, variable]));

  for (const name of delta.removed) {
    byName.delete(name);
  }
  for (const variable of [...delta.added, ...delta.changed]) {
    byName.set(variable.name, variable);
  }

  return [...byName.values()].sort((left, right) =>
    left.name.localeCompare(right.name),
  );
}

function diagnosticText(
  diagnostic: Extract<
    KernelEvent,
    { readonly event: { readonly type: "diagnostic" } }
  >["event"]["data"],
): string {
  const code = diagnostic.code === undefined ? "" : ` [${diagnostic.code}]`;
  const range =
    diagnostic.range === undefined
      ? ""
      : ` (${diagnostic.range.sourceName}:${diagnostic.range.start.toString()}-${diagnostic.range.end.toString()})`;
  return `${diagnostic.severity.toUpperCase()}${code}: ${diagnostic.message}${range}\n`;
}

function reduceKernelEvent(state: IdeState, event: KernelEvent): IdeState {
  switch (event.event.type) {
    case "status":
      return { ...state, kernelStatus: event.event.data.status };
    case "stream":
      return {
        ...state,
        commandWindowEntries: [
          ...state.commandWindowEntries,
          {
            id: event.messageId,
            channel: event.event.data.stream,
            text: event.event.data.text,
          },
        ],
      };
    case "display": {
      const representations = event.event.data.representations;
      if (representations[COMMAND_WINDOW_CLEAR_MIME] !== undefined) {
        return { ...state, commandWindowEntries: [] };
      }
      const isFigure =
        representations["application/vnd.openmat.plot+json"] !== undefined;
      const plainText = representations["text/plain"];
      return {
        ...state,
        figures: isFigure
          ? [...state.figures, event.event.data]
          : state.figures,
        commandWindowEntries:
          !isFigure && plainText !== undefined
            ? [
                ...state.commandWindowEntries,
                {
                  id: event.messageId,
                  channel: "stdout",
                  text: plainText.endsWith("\n") ? plainText : `${plainText}\n`,
                },
              ]
            : state.commandWindowEntries,
      };
    }
    case "diagnostic": {
      const channel =
        event.event.data.severity === "error" ? "stderr" : "system";
      return {
        ...state,
        error:
          event.event.data.severity === "error"
            ? event.event.data.message
            : state.error,
        commandWindowEntries: [
          ...state.commandWindowEntries,
          {
            id: event.messageId,
            channel,
            text: diagnosticText(event.event.data),
          },
        ],
      };
    }
    case "workspaceDelta": {
      const changedNames = new Set([
        ...event.event.data.changed.map((variable) => variable.name),
        ...event.event.data.removed,
      ]);
      return {
        ...state,
        workspace: applyWorkspaceDelta(state.workspace, event.event.data),
        inspection:
          state.inspection !== null && changedNames.has(state.inspection.name)
            ? null
            : state.inspection,
      };
    }
  }
}

function appendSystemMessage(state: IdeState, message: string): IdeState {
  return {
    ...state,
    commandWindowEntries: [
      ...state.commandWindowEntries,
      {
        id: `client-system-${(state.commandWindowEntries.length + 1).toString()}`,
        channel: "system",
        text: `${message}\n`,
      },
    ],
  };
}

export function ideReducer(state: IdeState, action: IdeAction): IdeState {
  switch (action.type) {
    case "connectionStarted":
      return {
        ...state,
        connection: "connecting",
        kernelStatus: "starting",
        negotiatedProtocol: null,
        capabilities: null,
        activeRequestId: null,
        figures: [],
        workspace: [],
        inspection: null,
        inspectingName: null,
        error: null,
      };
    case "connectionReady":
      return {
        ...state,
        connection: "connected",
        negotiatedProtocol: action.protocol,
        capabilities: action.capabilities,
        error: null,
      };
    case "connectionFailed":
      return {
        ...state,
        connection: "error",
        kernelStatus: "dead",
        negotiatedProtocol: null,
        capabilities: null,
        activeRequestId: null,
        inspectingName: null,
        error: action.message,
      };
    case "executionStarted":
      return {
        ...state,
        activeRequestId: action.requestId,
        inspection: null,
        error: null,
        commandWindowEntries:
          action.command === undefined
            ? state.commandWindowEntries
            : [
                ...state.commandWindowEntries,
                {
                  id: `${action.requestId}-input`,
                  channel: "input",
                  text: `>> ${action.command.trim()}\n`,
                },
              ],
      };
    case "executionFinished":
      return state.activeRequestId === action.requestId
        ? { ...state, activeRequestId: null }
        : state;
    case "executionFailed":
      return {
        ...state,
        activeRequestId: null,
        inspectingName: null,
        error: action.message,
        commandWindowEntries: [
          ...state.commandWindowEntries,
          {
            id: `client-error-${(state.commandWindowEntries.length + 1).toString()}`,
            channel: "stderr",
            text: `${action.message}\n`,
          },
        ],
      };
    case "workspaceLoaded":
      return {
        ...state,
        workspace: [...action.variables].sort((left, right) =>
          left.name.localeCompare(right.name),
        ),
        inspection: null,
      };
    case "inspectionStarted":
      return { ...state, inspectingName: action.name, error: null };
    case "inspectionLoaded":
      return {
        ...state,
        inspectingName: null,
        inspection: { name: action.name, preview: action.preview },
      };
    case "operationFailed":
      return {
        ...appendSystemMessage(state, action.message),
        inspectingName: null,
        error: action.message,
      };
    case "systemMessage":
      return appendSystemMessage(state, action.message);
    case "commandWindowCleared":
      return { ...state, commandWindowEntries: [] };
    case "figureClosed":
      return {
        ...state,
        figures: state.figures.filter((_figure, index) => index !== action.index),
      };
    case "eventReceived":
      return reduceKernelEvent(state, action.event);
  }
}
