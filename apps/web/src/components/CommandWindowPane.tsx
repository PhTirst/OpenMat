import {
  useEffect,
  useRef,
  type FormEvent,
  type KeyboardEvent,
  type MouseEvent,
} from "react";
import type { CommandWindowEntry } from "../state/ide-state";

interface CommandWindowPaneProps {
  readonly entries: readonly CommandWindowEntry[];
  readonly command: string;
  readonly history: readonly string[];
  readonly enabled: boolean;
  readonly canSubmit: boolean;
  readonly busy: boolean;
  readonly onCommandChange: (command: string) => void;
  readonly onSubmit: (command: string) => void;
  readonly onClear: () => void;
}

export function CommandWindowPane({
  entries,
  command,
  history,
  enabled,
  canSubmit,
  busy,
  onCommandChange,
  onSubmit,
  onClear,
}: CommandWindowPaneProps) {
  const endRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const historyIndexRef = useRef<number | null>(null);
  const draftBeforeHistoryRef = useRef("");

  useEffect(() => {
    endRef.current?.scrollIntoView({ block: "nearest" });
  }, [busy, entries.length]);

  const submit = (event?: FormEvent): void => {
    event?.preventDefault();
    if (!canSubmit || command.trim().length === 0) {
      return;
    }
    historyIndexRef.current = null;
    draftBeforeHistoryRef.current = "";
    onSubmit(command);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): void => {
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      submit();
      return;
    }
    if (event.key === "l" && event.ctrlKey) {
      event.preventDefault();
      onClear();
      return;
    }
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") {
      return;
    }
    if (history.length === 0 || command.includes("\n")) {
      return;
    }

    event.preventDefault();
    if (event.key === "ArrowUp") {
      if (historyIndexRef.current === null) {
        draftBeforeHistoryRef.current = command;
        historyIndexRef.current = history.length - 1;
      } else {
        historyIndexRef.current = Math.max(0, historyIndexRef.current - 1);
      }
      onCommandChange(history[historyIndexRef.current] ?? "");
      return;
    }

    if (historyIndexRef.current === null) {
      return;
    }
    if (historyIndexRef.current >= history.length - 1) {
      historyIndexRef.current = null;
      onCommandChange(draftBeforeHistoryRef.current);
    } else {
      historyIndexRef.current += 1;
      onCommandChange(history[historyIndexRef.current] ?? "");
    }
  };

  const focusPrompt = (event: MouseEvent<HTMLDivElement>): void => {
    if (event.target === event.currentTarget) {
      inputRef.current?.focus();
    }
  };

  return (
    <section
      className="pane command-window-pane"
      aria-labelledby="command-window-title"
    >
      <header className="pane-header">
        <span id="command-window-title" className="pane-title">
          Command Window
        </span>
        <button
          className="text-button"
          type="button"
          onClick={onClear}
          disabled={entries.length === 0}
        >
          Clear
        </button>
      </header>
      <div className="command-transcript" onClick={focusPrompt}>
        <div className="command-output" role="log" aria-live="polite">
          {entries.map((entry) => (
            <pre key={entry.id} className={`command-line ${entry.channel}`}>
              {entry.text}
            </pre>
          ))}
        </div>
        <form className="command-form" onSubmit={submit}>
          <label className="command-prompt" htmlFor="command-window-input">
            &gt;&gt;
          </label>
          <textarea
            ref={inputRef}
            id="command-window-input"
            className="command-input"
            aria-label="Command Window input"
            rows={1}
            value={command}
            disabled={!enabled}
            spellCheck={false}
            onChange={(event) => onCommandChange(event.target.value)}
            onKeyDown={handleKeyDown}
          />
        </form>
        <div ref={endRef} />
      </div>
    </section>
  );
}
