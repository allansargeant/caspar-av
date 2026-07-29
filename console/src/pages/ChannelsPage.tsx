import { useState } from "react";
import * as actions from "../lib/actions";
import { timecode, type ChannelState, type LayerState, type SlotState } from "../lib/types";
import { CommandLog, Field, Frame, Inspector } from "../shell/Shell";
import { run, ScreenPicker, type PageProps } from "./common";

/**
 * The live view: what the server is actually doing, straight from OSC.
 *
 * This is the page that proves the telemetry path works, so it stays honest
 * about the difference between "nothing is playing" and "no telemetry is
 * arriving" — they look identical otherwise, and they have completely
 * different fixes.
 */
export function ChannelsPage(props: PageProps) {
  const { snapshot, target } = props;
  const [selected, setSelected] = useState<string | null>(null);
  const [command, setCommand] = useState("");

  const channels = snapshot.channels;
  const selectedLayer = findLayer(channels, selected);

  const screenFor = (channel: number, layer: number) =>
    snapshot.show.screens.find((s) => s.channel === channel && s.layer === layer);

  const toolbar = (
    <>
      <span className="toolbar-title">Channels</span>
      {snapshot.server.osc_port != null && (
        <span className="chip">OSC :{snapshot.server.osc_port}</span>
      )}
      <span className="spacer" />
      <ScreenPicker {...props} />
      <div className="transport">
        <button className="tbtn" disabled={!target} onClick={() => target && run(actions.take(target))} title="Take">
          ▶
        </button>
        <button className="tbtn" disabled={!target} onClick={() => target && run(actions.pause(target))} title="Pause">
          ‖
        </button>
        <button className="tbtn" disabled={!target} onClick={() => target && run(actions.stop(target))} title="Stop">
          ■
        </button>
        <button className="tbtn" disabled={!target} onClick={() => target && run(actions.clear(target))} title="Clear">
          ✕
        </button>
      </div>
    </>
  );

  const centre =
    channels.length === 0 ? (
      <div className="canvas-wrap">
        <div className="list-empty">
          {snapshot.health !== "connected" ? (
            "Not connected to a CasparCG server"
          ) : snapshot.server.osc_port == null ? (
            <>
              <p>
                <strong>No OSC telemetry.</strong>
              </p>
              <p className="small">
                Commands still work — but CasparCG reports playback state only by pushing OSC,
                so nothing live can be shown until that port is reachable.
              </p>
            </>
          ) : (
            "Waiting for the first telemetry frame…"
          )}
        </div>
      </div>
    ) : (
      <div className="channels">
        {channels.map((ch) => (
          <div className="channel-card" key={ch.index}>
            <div className="channel-head">
              <span className="channel-index">CH {ch.index}</span>
              <span className="small muted">{ch.format ?? "—"}</span>
              <span className="spacer" />
              {ch.framerate != null && <span className="chip">{ch.framerate.toFixed(2)} fps</span>}
            </div>
            {ch.layers.length === 0 && <div className="list-empty small">No layers active</div>}
            {ch.layers.map((layer) => {
              const key = `${ch.index}-${layer.index}`;
              const screen = screenFor(ch.index, layer.index);
              return (
                <div
                  key={key}
                  className={`layer-row ${selected === key ? "sel" : ""}`}
                  onClick={() => setSelected(key)}
                >
                  <span className={`dot ${layer.foreground ? (layer.foreground.paused ? "paused" : "playing") : ""}`} />
                  <span className="layer-index">{layer.index}</span>
                  <div className="layer-body">
                    <span className="layer-clip">
                      {clipName(layer.foreground) ?? <span className="dim">empty</span>}
                      {screen && <span className="dim small"> · {screen.name}</span>}
                    </span>
                    <Progress slot={layer.foreground} />
                  </div>
                  <span className="layer-time">
                    {timecode(layer.foreground?.time)} / {timecode(layer.foreground?.duration)}
                  </span>
                </div>
              );
            })}
          </div>
        ))}
      </div>
    );

  const right = (
    <Inspector title="Layer" empty="Select a layer">
      {selectedLayer ? (
        <>
          <Field label="Channel">{selectedLayer.channel}</Field>
          <Field label="Layer">{selectedLayer.layer.index}</Field>
          <SlotFields title="Foreground" slot={selectedLayer.layer.foreground} />
          <SlotFields title="Background" slot={selectedLayer.layer.background} />
        </>
      ) : undefined}
    </Inspector>
  );

  // The command line is on this page because this is where you end up when
  // something is wrong and you need to talk to the server directly.
  const bottom = (
    <CommandLog snapshot={snapshot}>
      <form
        className="cmdline"
        onSubmit={(e) => {
          e.preventDefault();
          if (!command.trim()) return;
          run(actions.rawCommand(command.trim()));
          setCommand("");
        }}
      >
        <input
          placeholder="AMCP command, e.g. INFO or PLAY 1-10 AMB LOOP"
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          spellCheck={false}
        />
        <button className="btn" type="submit" disabled={snapshot.health !== "connected"}>
          Send
        </button>
      </form>
    </CommandLog>
  );

  return <Frame toolbar={toolbar} centre={centre} right={right} bottom={bottom} />;
}

function Progress({ slot }: { slot: SlotState | null }) {
  if (!slot || !slot.duration || !slot.time) return <div className="progress" />;
  const pct = Math.min(100, (slot.time / slot.duration) * 100);
  return (
    <div className="progress">
      <div className="progress-fill" style={{ width: `${pct}%` }} />
    </div>
  );
}

function SlotFields({ title, slot }: { title: string; slot: SlotState | null }) {
  return (
    <>
      <div className="inspector-sub">{title}</div>
      {slot ? (
        <>
          <Field label="Clip">
            <span className="mono small">{clipName(slot) ?? "—"}</span>
          </Field>
          <Field label="Producer">{slot.producer ?? "—"}</Field>
          <Field label="State">{slot.paused ? "paused" : "playing"}</Field>
          <Field label="Position">
            {timecode(slot.time)} / {timecode(slot.duration)}
          </Field>
          <Field label="Frame">
            {slot.frame != null ? `${Math.round(slot.frame)} / ${Math.round(slot.frames ?? 0)}` : "—"}
          </Field>
          <Field label="fps">{slot.fps?.toFixed(2) ?? "—"}</Field>
          <Field label="Size">
            {slot.width && slot.height ? `${slot.width}×${slot.height}` : "—"}
          </Field>
          <Field label="Loop">{slot.loop ? "yes" : "no"}</Field>
        </>
      ) : (
        <div className="small dim">empty</div>
      )}
    </>
  );
}

function clipName(slot: SlotState | null): string | null {
  if (!slot) return null;
  return slot.name ?? slot.path ?? slot.producer ?? null;
}

function findLayer(
  channels: ChannelState[],
  key: string | null,
): { channel: number; layer: LayerState } | null {
  if (!key) return null;
  const [ch, ly] = key.split("-").map(Number);
  const channel = channels.find((c) => c.index === ch);
  const layer = channel?.layers.find((l) => l.index === ly);
  return channel && layer ? { channel: channel.index, layer } : null;
}
