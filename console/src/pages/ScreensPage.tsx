import { useRef, useState } from "react";
import * as actions from "../lib/actions";
import type { Corners, Screen } from "../lib/types";
import { CommandLog, Field, Frame, Inspector } from "../shell/Shell";
import { run, type PageProps } from "./common";

const IDENTITY: Corners = [
  [0, 0],
  [1, 0],
  [1, 1],
  [0, 1],
];

/**
 * The output-mapping page.
 *
 * A screen is a Caspar layer with a rectangle on the show canvas. Dragging it
 * writes `MIXER FILL`; the corner-pin numbers write `MIXER PERSPECTIVE`. Both
 * are things CasparCG has always been able to do — what was missing was
 * somewhere to see them.
 */
export function ScreensPage({ snapshot, target, setTarget }: PageProps) {
  const show = snapshot.show;
  const screens = show.screens;
  const selected = screens.find((s) => s.id === target) ?? null;
  const [newName, setNewName] = useState("");

  const addScreen = () => {
    const name = newName.trim() || `Screen ${screens.length + 1}`;
    const id = actions.slug(name, screens.map((s) => s.id));
    // Default to a fresh channel rather than stacking on an existing layer,
    // which would silently replace another screen.
    const channel = Math.max(0, ...screens.map((s) => s.channel)) + 1;
    const screen: Screen = {
      id,
      name,
      channel,
      layer: 10,
      rect: { x: 0, y: 0, w: 1, h: 1 },
      corners: IDENTITY,
      enabled: true,
      opacity: 1,
    };
    setNewName("");
    setTarget(id);
    run(actions.addScreen(screen));
  };

  const update = (screen: Screen) => run(actions.updateScreen(screen));

  const toolbar = (
    <>
      <span className="toolbar-title">Screens</span>
      <span className="chip">
        canvas {show.canvas.width}×{show.canvas.height}
      </span>
      <span className="spacer" />
      <button className="btn tiny" onClick={() => run(actions.pushMapping())}>
        Re-send mapping
      </button>
    </>
  );

  const left = (
    <div className="panel">
      <div className="panel-head">Screens</div>
      <div className="addform">
        <input
          placeholder="New screen name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && addScreen()}
        />
        <button className="btn" onClick={addScreen}>
          Add screen
        </button>
      </div>
      <div className="list">
        {screens.length === 0 && <div className="list-empty">No screens yet</div>}
        {screens.map((s) => (
          <div
            key={s.id}
            className={`list-row ${s.id === target ? "sel" : ""} ${s.enabled ? "" : "off"}`}
            onClick={() => setTarget(s.id)}
          >
            <span className="dot" />
            <span className="row-name">{s.name}</span>
            <span className="mono small dim">
              {s.channel}-{s.layer}
            </span>
          </div>
        ))}
      </div>
    </div>
  );

  const centre = (
    <div className="canvas-wrap">
      <CanvasEditor
        screens={screens}
        canvas={show.canvas}
        selected={target}
        onSelect={setTarget}
        onChange={update}
      />
      <div className="canvas-hint dim">
        Drag to position · corner handle to resize · values write MIXER FILL live
      </div>
    </div>
  );

  const right = (
    <Inspector title="Screen" empty="Select a screen">
      {selected ? (
        <ScreenInspector
          screen={selected}
          onChange={update}
          onDelete={() => {
            setTarget(null);
            run(actions.deleteScreen(selected.id));
          }}
        />
      ) : undefined}
    </Inspector>
  );

  return (
    <Frame toolbar={toolbar} left={left} centre={centre} right={right} bottom={<CommandLog snapshot={snapshot} />} />
  );
}

function CanvasEditor(props: {
  screens: Screen[];
  canvas: { width: number; height: number };
  selected: string | null;
  onSelect: (id: string) => void;
  onChange: (s: Screen) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const aspect = props.canvas.width / Math.max(1, props.canvas.height);

  /**
   * Drag in canvas-normalised units.
   *
   * Pointer capture is what makes this reliable: without it a fast drag that
   * leaves the element drops the gesture, which on a mapping editor means a
   * screen left somewhere the operator did not intend.
   */
  const startDrag = (
    e: React.PointerEvent,
    screen: Screen,
    mode: "move" | "resize",
  ) => {
    e.preventDefault();
    e.stopPropagation();
    const box = ref.current?.getBoundingClientRect();
    if (!box) return;

    const startX = e.clientX;
    const startY = e.clientY;
    const origin = { ...screen.rect };
    const element = e.currentTarget as HTMLElement;
    element.setPointerCapture(e.pointerId);

    let latest = screen;

    const onMove = (ev: PointerEvent) => {
      const dx = (ev.clientX - startX) / box.width;
      const dy = (ev.clientY - startY) / box.height;
      const rect =
        mode === "move"
          ? {
              ...origin,
              x: clamp(origin.x + dx, -1, 2),
              y: clamp(origin.y + dy, -1, 2),
            }
          : {
              ...origin,
              w: Math.max(0.02, origin.w + dx),
              h: Math.max(0.02, origin.h + dy),
            };
      latest = { ...screen, rect };
      props.onChange(latest);
    };

    const onUp = () => {
      element.releasePointerCapture(e.pointerId);
      element.removeEventListener("pointermove", onMove);
      element.removeEventListener("pointerup", onUp);
      props.onChange(latest);
    };

    element.addEventListener("pointermove", onMove);
    element.addEventListener("pointerup", onUp);
  };

  return (
    <div className="canvas" ref={ref} style={{ aspectRatio: `${aspect}` }}>
      {props.screens.length === 0 && <div className="canvas-empty">Add a screen to map an output</div>}
      {props.screens.map((s) => (
        <div
          key={s.id}
          className={`viewport ${s.id === props.selected ? "sel" : ""} ${s.enabled ? "" : "disabled"}`}
          style={{
            left: `${s.rect.x * 100}%`,
            top: `${s.rect.y * 100}%`,
            width: `${s.rect.w * 100}%`,
            height: `${s.rect.h * 100}%`,
          }}
          onPointerDown={(e) => {
            props.onSelect(s.id);
            startDrag(e, s, "move");
          }}
        >
          <div className="viewport-label">
            <span className="mono small">
              {s.channel}-{s.layer}
            </span>
            <span>{s.name}</span>
          </div>
          <div className="viewport-resize" onPointerDown={(e) => startDrag(e, s, "resize")} />
        </div>
      ))}
    </div>
  );
}

function ScreenInspector(props: {
  screen: Screen;
  onChange: (s: Screen) => void;
  onDelete: () => void;
}) {
  const s = props.screen;
  const set = (patch: Partial<Screen>) => props.onChange({ ...s, ...patch });
  const setRect = (patch: Partial<Screen["rect"]>) => set({ rect: { ...s.rect, ...patch } });

  const warped = JSON.stringify(s.corners) !== JSON.stringify(IDENTITY);

  return (
    <>
      <div className="field-row">
        <span className="field-label">Name</span>
        <input value={s.name} onChange={(e) => set({ name: e.target.value })} style={{ width: 130 }} />
      </div>
      <div className="field-row">
        <span className="field-label">Channel</span>
        <input
          className="num"
          type="number"
          min={1}
          value={s.channel}
          onChange={(e) => set({ channel: Math.max(1, Number(e.target.value)) })}
        />
      </div>
      <div className="field-row">
        <span className="field-label">Layer</span>
        <input
          className="num"
          type="number"
          min={0}
          value={s.layer}
          onChange={(e) => set({ layer: Math.max(0, Number(e.target.value)) })}
        />
      </div>
      <div className="field-row">
        <span className="field-label">Enabled</span>
        <input type="checkbox" checked={s.enabled} onChange={(e) => set({ enabled: e.target.checked })} />
      </div>

      <div className="inspector-sub">Fill · MIXER FILL</div>
      {(["x", "y", "w", "h"] as const).map((k) => (
        <div className="field-row" key={k}>
          <span className="field-label">{k}</span>
          <input
            className="num"
            type="number"
            step={0.01}
            value={round(s.rect[k])}
            onChange={(e) => setRect({ [k]: Number(e.target.value) } as Partial<Screen["rect"]>)}
          />
        </div>
      ))}

      <div className="inspector-sub">Opacity</div>
      <div className="field-row">
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={s.opacity}
          onChange={(e) => set({ opacity: Number(e.target.value) })}
          style={{ flex: 1 }}
        />
        <span className="field-value mono">{s.opacity.toFixed(2)}</span>
      </div>

      <div className="inspector-sub">
        Corner pin · MIXER PERSPECTIVE {warped ? "" : "(identity)"}
      </div>
      {s.corners.map((corner, i) => (
        <div className="field-row" key={i}>
          <span className="field-label">{["TL", "TR", "BR", "BL"][i]}</span>
          <span className="row-inline">
            {[0, 1].map((axis) => (
              <input
                key={axis}
                className="num"
                style={{ width: 60 }}
                type="number"
                step={0.01}
                value={round(corner[axis])}
                onChange={(e) => {
                  const corners = s.corners.map((c) => [...c]) as Corners;
                  corners[i][axis] = Number(e.target.value);
                  set({ corners });
                }}
              />
            ))}
          </span>
        </div>
      ))}

      <div className="inspector-actions">
        <button className="btn" onClick={() => set({ corners: IDENTITY })} disabled={!warped}>
          Reset warp
        </button>
        <button
          className="btn"
          onClick={() => set({ rect: { x: 0, y: 0, w: 1, h: 1 } })}
          title="Fill the whole canvas"
        >
          Full frame
        </button>
        <button className="btn-danger" onClick={props.onDelete}>
          Delete
        </button>
      </div>
      <Field label="AMCP">
        <span className="mono small">
          {s.channel}-{s.layer}
        </span>
      </Field>
    </>
  );
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function round(v: number): number {
  return Math.round(v * 1000) / 1000;
}
