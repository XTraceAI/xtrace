import { useCallback, useLayoutEffect, useRef, useState } from 'react';
import { useLocation, useNavigate, useNavigationType } from 'react-router';

export function usePageHistory() {
  const { key } = useLocation();
  const action = useNavigationType();
  const navigate = useNavigate();
  const [history, setHistory] = useState({ keys: [key], index: 0 });
  const pending = useRef(false);

  // Only entries observed by this app are safe destinations. In particular,
  // the browser's earlier entries may belong to another site.
  if (history.keys[history.index] !== key) {
    if (action === 'PUSH') {
      setHistory({
        keys: [...history.keys.slice(0, history.index + 1), key],
        index: history.index + 1,
      });
    } else if (action === 'REPLACE') {
      setHistory({
        keys: history.keys.map((entry, i) => (i === history.index ? key : entry)),
        index: history.index,
      });
    } else {
      const index = history.keys.indexOf(key);
      setHistory(index < 0 ? { keys: [key], index: 0 } : { ...history, index });
    }
  }

  useLayoutEffect(() => {
    pending.current = false;
  }, [key]);

  const canGoBack = history.index > 0;
  const canGoForward = history.index < history.keys.length - 1;
  const move = useCallback(
    (direction: -1 | 1) => {
      if (pending.current || !(direction === -1 ? canGoBack : canGoForward)) return;
      // Browser POP is asynchronous: repeated clicks must not cross the boundary.
      pending.current = true;
      void navigate(direction);
    },
    [canGoBack, canGoForward, navigate],
  );
  const goBack = useCallback(() => move(-1), [move]);
  const goForward = useCallback(() => move(1), [move]);
  return { canGoBack, canGoForward, goBack, goForward };
}
