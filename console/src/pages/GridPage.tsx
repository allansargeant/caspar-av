import { useEffect, useState } from "react";
import * as actions from "../lib/actions";
import type { Pad } from "../lib/types";
import { CommandLog, Frame, Inspector } from "../shell/Shell";
import { run, type PageProps } from "./common";

/**
 * The trigger grid — cues as pads, for running a show by hand.
 *
 * Number keys 1–9 and 0 fire the first ten pads. That is deliberately the only
 * keyboard binding: an operator learns one row and trusts it, and a grid that
 * responds to every key is a grid that fires something during a name entry.
 */
export function GridPage({ snapshot }: PageProps) {
  const show = snapshot.show;
  const [cols, rows] = show.grid;
  const total = cols * rows;
  const [firing, setFiring] = useState<number | null>(null);
  const [assigning, setAssigning] = useState<number | null>(null);

  const padAt = (index: number) => show.pads.find((p) => p.index === index) ?? null;
  const cueOf = (pad: Pad | null) => (pad ? show.cues.find((c) => c.id === pad.cue) ?? null : null);

  const fire = (index: number) => {
    const cue = cueOf(padAt(index));
    if (!cue) return;
    setFiring(index);
    setTimeout(() => setFiring((f) => (f === index ? null : f)), 220);
    run(actions.fireCue(cue.id));
  };

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // Never steal a keystroke that is going into a field.
      const el = e.target as HTMLElement | null;
      if (el && ["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName)) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      if (!/^[0-9]$/.test(e.key)) return;
      const index = e.key === "0" ? 9 : Number(e.key) - 1;
      if (index < total) fire(index);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const assign = (index: number, cueId: string) => {
    const pads = show.pads.filter((p) => p.index !== index);
    if (cueId) pads.push({ index, cue: cueId });
    setAssigning(null);
    run(actions.setPads(pads));
  };

  const toolbar = (
    <>
      <span className="toolbar-title">Grid</span>
      <span className="chip">
        {cols} × {rows}
      </span>
      <span className="spacer" />
      <span className="small dim">Keys 1–9, 0 fire the first ten pads</span>
    </>
  );

  const centre = (
    <div className="grid-pads" style={{ gridTemplateColumns: `repeat(${cols}, minmax(110px, 1fr))` }}>
      {Array.from({ length: total }, (_, index) => {
        const pad = padAt(index);
        const cue = cueOf(pad);
        return (
          <button
            key={index}
            className={`gpad ${cue ? "" : "empty"} ${firing === index ? "firing" : ""}`}
            onClick={() => (cue ? fire(index) : setAssigning(index))}
            onContextMenu={(e) => {
              e.preventDefault();
              setAssigning(index);
            }}
            title={cue ? `Fire ${cue.name}` : "Assign a cue"}
          >
            <span className="gpad-key">
              {index < 10 ? (index === 9 ? "0" : index + 1) : index + 1}
            </span>
            <span className="gpad-name">{cue ? cue.name : "—"}</span>
          </button>
        );
      })}
    </div>
  );

  const right = (
    <Inspector title="Pad" empty="Click an empty pad to assign a cue">
      {assigning != null ? (
        <>
          <div className="field-row">
            <span className="field-label">Pad</span>
            <span className="field-value mono">{assigning + 1}</span>
          </div>
          <div className="inspector-sub">Cue</div>
          <select
            value={padAt(assigning)?.cue ?? ""}
            onChange={(e) => assign(assigning, e.target.value)}
            style={{ width: "100%" }}
          >
            <option value="">— none —</option>
            {show.cues.map((c) => (
              <option key={c.id} value={c.id}>
                {c.name}
              </option>
            ))}
          </select>
          {show.cues.length === 0 && (
            <div className="small dim" style={{ marginTop: 8 }}>
              No cues yet — build one on the Cues page first.
            </div>
          )}
          <div className="inspector-actions">
            <button className="btn" onClick={() => setAssigning(null)}>
              Done
            </button>
          </div>
        </>
      ) : undefined}
    </Inspector>
  );

  return <Frame toolbar={toolbar} centre={centre} right={right} bottom={<CommandLog snapshot={snapshot} />} />;
}
