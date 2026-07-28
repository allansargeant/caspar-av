import { useMemo, useState } from "react";
import * as actions from "../lib/actions";
import { mediaKind, timecode, type MediaItem } from "../lib/types";
import { CommandLog, Field, Frame, Inspector } from "../shell/Shell";
import { fileSize, run, ScreenPicker, type PageProps } from "./common";

const KINDS = ["all", "movie", "still", "audio"] as const;
type Kind = (typeof KINDS)[number];

export function MediaPage(props: PageProps) {
  const { snapshot, target } = props;
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<Kind>("all");
  const [selected, setSelected] = useState<string | null>(null);
  const [looping, setLooping] = useState(false);
  const [frames, setFrames] = useState(0);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    return snapshot.media
      .filter((m) => (kind === "all" ? true : mediaKind(m) === kind))
      .filter((m) => (q ? m.name.toLowerCase().includes(q) : true))
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [snapshot.media, query, kind]);

  const item = items.find((m) => m.name === selected) ?? null;

  const toolbar = (
    <>
      <span className="toolbar-title">Media</span>
      <input
        placeholder="Search…"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
        style={{ width: 180 }}
      />
      <select value={kind} onChange={(e) => setKind(e.target.value as Kind)}>
        {KINDS.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </select>
      <span className="spacer" />
      <ScreenPicker {...props} />
      <button className="btn tiny" onClick={() => run(actions.refreshLibrary())}>
        Rescan
      </button>
    </>
  );

  // media-scanner is a separate process, and a stopped one looks exactly like
  // an empty media folder. Saying which it is saves the classic hour.
  const centre = !snapshot.scanner_up ? (
    <div className="canvas-wrap">
      <div className="list-empty">
        <p>
          <strong>media-scanner is not responding.</strong>
        </p>
        <p className="small">
          CasparCG 2.5 has no media list of its own — <span className="mono">CLS</span>,{" "}
          <span className="mono">TLS</span> and every thumbnail are proxied to the scanner
          service. Start it, then press Rescan.
        </p>
      </div>
    </div>
  ) : items.length === 0 ? (
    <div className="canvas-wrap">
      <div className="list-empty">
        {snapshot.media.length === 0 ? "No media found" : "Nothing matches that filter"}
      </div>
    </div>
  ) : (
    <div className="media-grid">
      {items.map((m) => (
        <MediaTile
          key={m.name}
          item={m}
          selected={m.name === selected}
          onSelect={() => setSelected(m.name)}
          onPlay={() => target && run(actions.play(target, m.name, looping, frames))}
        />
      ))}
    </div>
  );

  const right = (
    <Inspector title="Clip" empty="Select a clip">
      {item ? (
        <>
          <Field label="Name">
            <span className="mono">{item.name}</span>
          </Field>
          <Field label="Kind">{mediaKind(item)}</Field>
          <Field label="Duration">{timecode(Number(item.format?.duration ?? NaN))}</Field>
          <Field label="Resolution">{resolution(item)}</Field>
          <Field label="Container">{item.format?.name ?? "—"}</Field>
          <Field label="Size">{fileSize(item.mediaSize)}</Field>

          <div className="inspector-sub">Playback</div>
          <div className="field-row">
            <span className="field-label">Loop</span>
            <input type="checkbox" checked={looping} onChange={(e) => setLooping(e.target.checked)} />
          </div>
          <div className="field-row">
            <span className="field-label">Mix (frames)</span>
            <input
              className="num"
              type="number"
              min={0}
              value={frames}
              onChange={(e) => setFrames(Math.max(0, Number(e.target.value)))}
            />
          </div>

          <div className="inspector-actions">
            <button
              className="btn-primary"
              disabled={!target}
              onClick={() => target && run(actions.play(target, item.name, looping, frames))}
            >
              Play
            </button>
            <button
              className="btn"
              disabled={!target}
              title="Cue into the background, ready to take"
              onClick={() => target && run(actions.load(target, item.name, looping, frames))}
            >
              Cue
            </button>
            <button className="btn" disabled={!target} onClick={() => target && run(actions.take(target))}>
              Take
            </button>
          </div>
          {!target && <div className="small dim">Add a screen to play onto.</div>}
        </>
      ) : undefined}
    </Inspector>
  );

  return <Frame toolbar={toolbar} centre={centre} right={right} bottom={<CommandLog snapshot={snapshot} />} />;
}

function resolution(item: MediaItem): string {
  const stream = (item.streams ?? []).find((s) => s.width && s.height);
  return stream ? `${stream.width}×${stream.height}` : "—";
}

function MediaTile(props: {
  item: MediaItem;
  selected: boolean;
  onSelect: () => void;
  onPlay: () => void;
}) {
  const { item } = props;
  const kind = mediaKind(item);
  const [broken, setBroken] = useState(false);

  return (
    <button
      className={`media-tile ${props.selected ? "sel" : ""}`}
      onClick={props.onSelect}
      onDoubleClick={props.onPlay}
      title={`${item.name} — double-click to play`}
    >
      <div className="media-thumb">
        {kind === "audio" || broken ? (
          <span>{kind === "audio" ? "♪" : "▦"}</span>
        ) : (
          <img
            src={actions.thumbnailUrl(item.name)}
            alt=""
            loading="lazy"
            onError={() => setBroken(true)}
          />
        )}
      </div>
      <div className="media-meta">
        <span className="media-name">{item.name}</span>
        <span className="media-sub">
          <span className={`kind ${kind}`}>{kind}</span>
          <span>{timecode(Number(item.format?.duration ?? NaN))}</span>
        </span>
      </div>
    </button>
  );
}
