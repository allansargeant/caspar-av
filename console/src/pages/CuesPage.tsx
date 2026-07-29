import { useEffect, useState } from "react";
import * as actions from "../lib/actions";
import type { Action, Cue } from "../lib/types";
import { CommandLog, Frame, Inspector } from "../shell/Shell";
import { run, type PageProps } from "./common";

/**
 * Cues: several screens changed together.
 *
 * The daemon fires a cue as one `BEGIN`/`COMMIT` batch, which is why this is
 * worth having over a list of buttons — the server locks every touched channel
 * and releases them on the same frame, so a three-screen change lands as one
 * change rather than three.
 */
export function CuesPage({ snapshot, target, setTarget }: PageProps) {
  const cues = snapshot.show.cues;
  const screens = snapshot.show.screens;
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [newName, setNewName] = useState("");

  const cue = cues.find((c) => c.id === selectedId) ?? null;

  // Land on the first cue rather than two "select something" panels. Also
  // recovers the selection when the chosen cue is deleted by another console.
  useEffect(() => {
    if (cues.length > 0 && !cues.some((c) => c.id === selectedId)) {
      setSelectedId(cues[0].id);
    }
  }, [cues, selectedId]);

  const addCue = () => {
    const name = newName.trim() || `Cue ${cues.length + 1}`;
    const id = actions.slug(name, cues.map((c) => c.id));
    setNewName("");
    setSelectedId(id);
    run(actions.addCue({ id, name, actions: [], follow: null, colour: null }));
  };

  const update = (next: Cue) => run(actions.updateCue(next));

  const addAction = (action: Action) => {
    if (!cue) return;
    update({ ...cue, actions: [...cue.actions, action] });
  };

  const toolbar = (
    <>
      <span className="toolbar-title">Cues</span>
      <span className="chip">{cues.length} cues</span>
      <span className="spacer" />
      {cue && (
        <button className="btn-primary" onClick={() => run(actions.fireCue(cue.id))}>
          Fire “{cue.name}”
        </button>
      )}
    </>
  );

  const left = (
    <div className="panel">
      <div className="panel-head">Cue list</div>
      <div className="addform">
        <input
          placeholder="New cue name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && addCue()}
        />
        <button className="btn" onClick={addCue}>
          Add cue
        </button>
      </div>
      <div className="list">
        {cues.length === 0 && <div className="list-empty">No cues yet</div>}
        {cues.map((c, i) => (
          <div
            key={c.id}
            className={`list-row ${c.id === selectedId ? "sel" : ""}`}
            onClick={() => setSelectedId(c.id)}
          >
            <span className="cue-num mono">{i + 1}</span>
            <span className="row-name">{c.name}</span>
            <span className="dim small">{c.actions.length}</span>
            <button
              className="row-fire"
              onClick={(e) => {
                e.stopPropagation();
                run(actions.fireCue(c.id));
              }}
            >
              fire
            </button>
          </div>
        ))}
      </div>
    </div>
  );

  const centre = (
    <div className="canvas-wrap">
      {!cue ? (
        <div className="list-empty">Select or add a cue</div>
      ) : cue.actions.length === 0 ? (
        <div className="list-empty">
          This cue does nothing yet — add actions from the panel on the right.
        </div>
      ) : (
        <div className="stack">
          {cue.actions.map((a, i) => (
            <div className="list-row" key={i} style={{ borderBottom: "1px solid var(--line)" }}>
              <span className="cue-num mono">{i + 1}</span>
              <span className="row-name">{describe(a, screens)}</span>
              <button
                className="btn tiny btn-danger"
                onClick={() => update({ ...cue, actions: cue.actions.filter((_, j) => j !== i) })}
              >
                remove
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );

  const right = (
    <Inspector title="Cue" empty="Select a cue">
      {cue ? (
        <>
          <div className="field-row">
            <span className="field-label">Name</span>
            <input
              value={cue.name}
              onChange={(e) => update({ ...cue, name: e.target.value })}
              style={{ width: 130 }}
            />
          </div>
          <div className="field-row">
            <span className="field-label">Auto-follow (s)</span>
            <input
              className="num"
              type="number"
              min={0}
              step={0.5}
              value={cue.follow ?? 0}
              onChange={(e) => {
                const v = Number(e.target.value);
                update({ ...cue, follow: v > 0 ? v : null });
              }}
            />
          </div>

          <div className="inspector-sub">Add action</div>
          <AddAction screens={screens} target={target} setTarget={setTarget} onAdd={addAction} />

          <div className="inspector-actions">
            <button className="btn-primary" onClick={() => run(actions.fireCue(cue.id))}>
              Fire
            </button>
            <button
              className="btn-danger"
              onClick={() => {
                setSelectedId(null);
                run(actions.deleteCue(cue.id));
              }}
            >
              Delete cue
            </button>
          </div>
        </>
      ) : undefined}
    </Inspector>
  );

  return (
    <Frame toolbar={toolbar} left={left} centre={centre} right={right} bottom={<CommandLog snapshot={snapshot} />} />
  );
}

const SIMPLE = ["take", "pause", "resume", "stop", "clear", "remap"] as const;

function AddAction(props: {
  screens: PageProps["snapshot"]["show"]["screens"];
  target: string | null;
  setTarget: (id: string) => void;
  onAdd: (a: Action) => void;
}) {
  const [kind, setKind] = useState<string>("play");
  const [clip, setClip] = useState("");
  const [looping, setLooping] = useState(false);
  const [frames, setFrames] = useState(0);
  const [value, setValue] = useState(1);
  const [raw, setRaw] = useState("");
  const screen = props.target ?? props.screens[0]?.id ?? "";

  const add = () => {
    if (kind === "raw") {
      if (!raw.trim()) return;
      props.onAdd({ type: "raw", command: raw.trim() });
      setRaw("");
      return;
    }
    if (!screen) return;
    if (kind === "play" || kind === "load") {
      if (!clip.trim()) return;
      props.onAdd({
        type: kind,
        screen,
        clip: clip.trim(),
        looping,
        transition: frames > 0 ? { kind: "mix", frames, tween: null, direction: null, sting: null } : null,
      });
      return;
    }
    if (kind === "opacity" || kind === "volume") {
      props.onAdd({ type: kind, screen, value, frames, tween: null });
      return;
    }
    props.onAdd({ type: kind, screen } as Action);
  };

  return (
    <div className="gdd-form">
      <select value={kind} onChange={(e) => setKind(e.target.value)}>
        <option value="play">Play clip</option>
        <option value="load">Cue clip (LOADBG)</option>
        {SIMPLE.map((s) => (
          <option key={s} value={s}>
            {s}
          </option>
        ))}
        <option value="opacity">Opacity</option>
        <option value="volume">Volume</option>
        <option value="raw">Raw AMCP</option>
      </select>

      {kind !== "raw" && (
        <select value={screen} onChange={(e) => props.setTarget(e.target.value)}>
          {props.screens.map((s) => (
            <option key={s.id} value={s.id}>
              {s.name}
            </option>
          ))}
        </select>
      )}

      {(kind === "play" || kind === "load") && (
        <>
          <input placeholder="Clip name" value={clip} onChange={(e) => setClip(e.target.value)} />
          <label className="row-inline small muted">
            <input type="checkbox" checked={looping} onChange={(e) => setLooping(e.target.checked)} />
            loop
          </label>
        </>
      )}

      {(kind === "opacity" || kind === "volume") && (
        <input
          className="num"
          type="number"
          step={0.05}
          min={0}
          max={1}
          value={value}
          onChange={(e) => setValue(Number(e.target.value))}
        />
      )}

      {(kind === "play" || kind === "load" || kind === "opacity" || kind === "volume") && (
        <label className="row-inline small muted">
          <input
            className="num"
            type="number"
            min={0}
            value={frames}
            onChange={(e) => setFrames(Math.max(0, Number(e.target.value)))}
          />
          frames
        </label>
      )}

      {kind === "raw" && (
        <input
          placeholder="e.g. CLEAR 1"
          className="mono"
          value={raw}
          onChange={(e) => setRaw(e.target.value)}
        />
      )}

      <button className="btn" onClick={add}>
        Add to cue
      </button>
    </div>
  );
}

function describe(a: Action, screens: PageProps["snapshot"]["show"]["screens"]): string {
  const name = (id: string) => screens.find((s) => s.id === id)?.name ?? id;
  switch (a.type) {
    case "play":
      return `Play “${a.clip}” on ${name(a.screen)}${a.looping ? " (loop)" : ""}${
        a.transition ? ` · mix ${a.transition.frames}` : ""
      }`;
    case "load":
      return `Cue “${a.clip}” on ${name(a.screen)}`;
    case "opacity":
      return `Opacity ${a.value} on ${name(a.screen)}${a.frames ? ` over ${a.frames}f` : ""}`;
    case "volume":
      return `Volume ${a.value} on ${name(a.screen)}${a.frames ? ` over ${a.frames}f` : ""}`;
    case "template":
      return `Template “${a.template}” on ${name(a.screen)}`;
    case "templatestop":
      return `Stop template on ${name(a.screen)}`;
    case "raw":
      return `AMCP: ${a.command}`;
    default:
      return `${a.type} ${name(a.screen)}`;
  }
}
