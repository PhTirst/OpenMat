import { useEffect, useRef, useState } from "react";
export function CommitField({
    value,
    onCommit,
    name,
    type = "text",
}: {
    value: string;
    onCommit(value: string): void | boolean;
    name: string;
    type?: string;
}) {
    const [draft, setDraft] = useState(value);
    const cancelled = useRef(false);
    useEffect(() => setDraft(value), [value]);
    return (
        <input
            aria-label={name}
            type={type}
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            onBlur={() => {
                if (
                    !cancelled.current &&
                    draft !== value &&
                    onCommit(draft) === false
                )
                    setDraft(value);
                cancelled.current = false;
            }}
            onKeyDown={(event) => {
                if (event.key === "Enter") event.currentTarget.blur();
                if (event.key === "Escape") {
                    cancelled.current = true;
                    setDraft(value);
                    event.currentTarget.blur();
                }
            }}
        />
    );
}
