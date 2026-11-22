// Live input stream (Observatory) as a tiny pub/sub, outside React state so
// high-rate input never re-renders the whole app.
import type { LiveInput } from "./types";

type Listener = (batch: LiveInput[]) => void;

const listeners = new Set<Listener>();

export const liveBus = {
  emit(batch: LiveInput[]) {
    listeners.forEach((l) => l(batch));
  },
  on(l: Listener) {
    listeners.add(l);
    return () => {
      listeners.delete(l);
    };
  },
};
