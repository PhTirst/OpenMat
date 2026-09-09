import { useCallback, useLayoutEffect, useRef, useState } from "react";

/** Exit checks must see the committed editor and designer state after async saves. */
export function useCommitBarrier(): () => Promise<void> {
  const [revision, setRevision] = useState(0);
  const waiting = useRef<Array<() => void>>([]);
  useLayoutEffect(() => {
    const completed = waiting.current;
    waiting.current = [];
    completed.forEach((resolve) => resolve());
  }, [revision]);
  return useCallback(() => new Promise<void>((resolve) => {
    waiting.current.push(resolve);
    setRevision((current) => current + 1);
  }), []);
}
