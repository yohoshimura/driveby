import { useEffect } from 'react';

export function useKeyboard(bindings) {
  useEffect(() => {
    const handler = (e) => {
      for (const { key, ctrl, meta, shift, handler: cb } of bindings) {
        const keyMatch = e.key.toLowerCase() === key.toLowerCase();
        const ctrlMatch = ctrl === undefined || ctrl === (e.ctrlKey || e.metaKey);
        const metaMatch = meta === undefined || meta === e.metaKey;
        const shiftMatch = shift === undefined || shift === e.shiftKey;
        if (keyMatch && ctrlMatch && metaMatch && shiftMatch) {
          e.preventDefault();
          cb(e);
          return;
        }
      }
    };
    window.addEventListener('keydown', handler);
    return () => window.removeEventListener('keydown', handler);
  }, [bindings]);
}
