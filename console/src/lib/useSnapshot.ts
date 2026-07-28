import { useEffect, useState } from "react";
import { connect, onApiError, type ConnStatus } from "./connection";
import { EMPTY, type Snapshot } from "./types";

/** Subscribe to the daemon's snapshot for the life of the component. */
export function useSnapshot(): { snapshot: Snapshot; status: ConnStatus; error: string | null } {
  const [snapshot, setSnapshot] = useState<Snapshot>(EMPTY);
  const [status, setStatus] = useState<ConnStatus>("connecting");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => connect({ onSnapshot: setSnapshot, onStatus: setStatus }), []);
  useEffect(() => onApiError(setError), []);

  return { snapshot, status, error };
}
