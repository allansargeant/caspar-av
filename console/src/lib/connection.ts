// Live connection to the daemon.
//
// Ported from OpenStage's console, which proved this shape against a live
// orchestrator: a WebSocket for snapshots, falling back to polling when the
// socket can't be kept up (a proxy that won't forward upgrades — common on a
// show network behind someone else's kit). The console only *reads* state here;
// changes go through `api()`.

import type { Snapshot } from "./types";

export type ConnStatus = "connecting" | "live" | "polling" | "down";

export interface ConnectionEvents {
  onSnapshot: (snap: Snapshot) => void;
  onStatus: (status: ConnStatus) => void;
}

/** Open a live connection. Returns a disposer that tears everything down. */
export function connect({ onSnapshot, onStatus }: ConnectionEvents): () => void {
  let ws: WebSocket | null = null;
  let pollTimer: ReturnType<typeof setInterval> | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let failures = 0;
  let disposed = false;
  let lastJson = "";

  const apply = (json: string) => {
    if (json === lastJson) return; // nothing changed
    lastJson = json;
    try {
      onSnapshot(JSON.parse(json) as Snapshot);
    } catch {
      /* ignore a malformed frame */
    }
  };

  const startPolling = () => {
    if (pollTimer || disposed) return;
    onStatus("polling");
    const poll = async () => {
      try {
        const r = await fetch("api/state", { cache: "no-store" });
        if (r.ok) apply(await r.text());
      } catch {
        onStatus("down");
      }
    };
    void poll();
    pollTimer = setInterval(poll, 1000);
  };

  const stopPolling = () => {
    if (pollTimer) {
      clearInterval(pollTimer);
      pollTimer = null;
    }
  };

  const openSocket = () => {
    if (disposed) return;
    const proto = location.protocol === "https:" ? "wss" : "ws";
    try {
      ws = new WebSocket(`${proto}://${location.host}/ws/ui`);
    } catch {
      failures++;
      startPolling();
      reconnectTimer = setTimeout(openSocket, 10_000);
      return;
    }

    ws.onopen = () => {
      failures = 0;
      stopPolling();
      onStatus("live");
    };
    ws.onmessage = (ev) => apply(ev.data as string);
    ws.onclose = () => {
      if (disposed) return;
      failures++;
      if (failures >= 3) {
        // Repeatedly failing: keep the console usable via polling, and retry
        // the socket occasionally in case whatever broke it clears up.
        startPolling();
        reconnectTimer = setTimeout(openSocket, 10_000);
      } else {
        onStatus("down");
        reconnectTimer = setTimeout(openSocket, 1500);
      }
    };
  };

  onStatus("connecting");
  openSocket();

  return () => {
    disposed = true;
    stopPolling();
    if (reconnectTimer) clearTimeout(reconnectTimer);
    if (ws) {
      ws.onclose = null; // don't trigger the reconnect path on a deliberate close
      ws.close();
    }
  };
}

/** The last error any `api()` call produced, for the status bar to surface. */
let lastError: string | null = null;
const errorListeners = new Set<(e: string | null) => void>();

export function onApiError(fn: (e: string | null) => void): () => void {
  errorListeners.add(fn);
  fn(lastError);
  return () => errorListeners.delete(fn);
}

function setError(e: string | null) {
  lastError = e;
  errorListeners.forEach((fn) => fn(e));
}

/**
 * Send a command to the daemon. Throws on a non-2xx, and publishes the error
 * so the shell can show it — a refused AMCP command needs to be *seen*, not
 * swallowed, because the server's own wording is usually the whole diagnosis.
 */
export async function api<T = unknown>(
  method: "GET" | "POST" | "PUT" | "PATCH" | "DELETE",
  path: string,
  body?: unknown,
): Promise<T> {
  const res = await fetch(`api${path}`, {
    method,
    headers: body !== undefined ? { "Content-Type": "application/json" } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  if (!res.ok) {
    let detail = text;
    try {
      detail = (JSON.parse(text) as { error?: string }).error ?? text;
    } catch {
      /* keep the raw body */
    }
    const message = `${path}: ${detail || res.status}`;
    setError(message);
    throw new Error(message);
  }
  setError(null);
  return text ? (JSON.parse(text) as T) : (undefined as T);
}
