import { useEffect, useRef, useState } from 'react';

// Keeps a child mounted long enough for an exit animation to play.
// Returns { mounted, state } where state is 'enter' | 'exit'.
// Pair with CSS that animates `[data-state="enter"]` and `[data-state="exit"]`.
export function useExitTransition(open, durationMs = 220) {
  const [mounted, setMounted] = useState(open);
  const [state, setState] = useState(open ? 'enter' : 'exit');
  const timer = useRef(null);

  useEffect(() => {
    if (open) {
      if (timer.current) { clearTimeout(timer.current); timer.current = null; }
      setMounted(true);
      requestAnimationFrame(() => setState('enter'));
    } else if (mounted) {
      setState('exit');
      timer.current = setTimeout(() => {
        setMounted(false);
        timer.current = null;
      }, durationMs);
    }
    return () => {
      if (timer.current) { clearTimeout(timer.current); timer.current = null; }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  return { mounted, state };
}
