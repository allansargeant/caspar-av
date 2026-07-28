import type { Snapshot } from "../lib/types";

/** What every page receives. */
export interface PageProps {
  snapshot: Snapshot;
  /** The screen most actions target. */
  target: string | null;
  setTarget: (id: string | null) => void;
}

/**
 * The screen chooser that appears in most page toolbars.
 *
 * Shown even when there are no screens, with an explanation — a disabled
 * control with no reason given is the thing that makes a tool feel broken.
 */
export function ScreenPicker({ snapshot, target, setTarget }: PageProps) {
  const screens = snapshot.show.screens;
  if (screens.length === 0) {
    return <span className="dim small">No screens yet — add one on the Screens page</span>;
  }
  return (
    <>
      <span className="small muted">Target</span>
      <select value={target ?? ""} onChange={(e) => setTarget(e.target.value)}>
        {screens.map((s) => (
          <option key={s.id} value={s.id}>
            {s.name} · {s.channel}-{s.layer}
          </option>
        ))}
      </select>
    </>
  );
}

/** Bytes as a short human string. */
export function fileSize(bytes: number | null | undefined): string {
  if (bytes == null) return "—";
  const units = ["B", "kB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}

/** Fire an action and swallow the rejection — `api()` already surfaced it. */
export function run(promise: Promise<unknown>): void {
  void promise.catch(() => {});
}
