interface CommandHistoryPaneProps {
  readonly commands: readonly string[];
  readonly onRecall: (command: string) => void;
  readonly onExecute: (command: string) => void;
}

export function CommandHistoryPane({
  commands,
  onRecall,
  onExecute,
}: CommandHistoryPaneProps) {
  return (
    <section
      className="pane command-history-pane"
      aria-labelledby="command-history-title"
    >
      <header className="pane-header">
        <span id="command-history-title" className="pane-title">
          Command History
        </span>
        <span className="pane-meta">
          {commands.length.toString()} {commands.length === 1 ? "command" : "commands"}
        </span>
      </header>
      {commands.length === 0 ? (
        <div className="empty-state">
          <span className="empty-symbol" aria-hidden="true">
            ↶
          </span>
          <strong>No commands yet</strong>
          <span>Commands submitted at the prompt will appear here.</span>
        </div>
      ) : (
        <ol className="command-history-list">
          {commands.map((command, index) => (
            <li key={`${index.toString()}-${command}`}>
              <button
                type="button"
                title="Copy to Command Window"
                onClick={() => onRecall(command)}
                onDoubleClick={() => onExecute(command)}
              >
                {command}
              </button>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
