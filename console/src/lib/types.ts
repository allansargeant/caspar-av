// The snapshot the daemon serves. Mirrors `Snapshot` in crates/showd/src/bridge.rs.
//
// The console is a passive mirror: nothing here is ever the source of truth.
// State comes down in a snapshot; changes go up as commands.

export type Health = "connecting" | "connected" | "down";

export interface SlotState {
  producer: string | null;
  paused: boolean;
  path: string | null;
  name: string | null;
  time: number | null;
  duration: number | null;
  frame: number | null;
  frames: number | null;
  fps: number | null;
  loop: boolean;
  width: number | null;
  height: number | null;
}

export interface LayerState {
  index: number;
  foreground: SlotState | null;
  background: SlotState | null;
}

export interface ChannelState {
  index: number;
  format: string | null;
  framerate: number | null;
  profiler_time: number | null;
  layers: LayerState[];
}

export interface MediaItem {
  name: string;
  path: string | null;
  mediaSize?: number | null;
  mediaTime?: number | null;
  format?: { name?: string; long_name?: string; duration?: string } | null;
  streams?: Array<{
    codec?: { type?: string; name?: string } | null;
    width?: number | null;
    height?: number | null;
    duration?: string | null;
    channels?: number | null;
  }>;
  [extra: string]: unknown;
}

export interface Template {
  id: string;
  path: string | null;
  type: string | null;
  /** A JSON Schema the template published about itself; drives the data form. */
  gdd: GddSchema | null;
  error: string | null;
}

/** The subset of GDD/JSON Schema the template form understands. */
export interface GddSchema {
  type?: string;
  title?: string;
  description?: string;
  properties?: Record<string, GddSchema>;
  required?: string[];
  default?: unknown;
  enum?: string[];
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export type Corners = [[number, number], [number, number], [number, number], [number, number]];

export interface Screen {
  id: string;
  name: string;
  channel: number;
  layer: number;
  rect: Rect;
  corners: Corners;
  enabled: boolean;
  opacity: number;
}

export interface TransitionSpec {
  kind: string;
  frames: number;
  tween: string | null;
  direction: string | null;
  sting: string | null;
}

export type Action =
  | { type: "play"; screen: string; clip: string; looping: boolean; transition: TransitionSpec | null }
  | { type: "load"; screen: string; clip: string; looping: boolean; transition: TransitionSpec | null }
  | { type: "take"; screen: string }
  | { type: "pause"; screen: string }
  | { type: "resume"; screen: string }
  | { type: "stop"; screen: string }
  | { type: "clear"; screen: string }
  | { type: "opacity"; screen: string; value: number; frames: number; tween: string | null }
  | { type: "volume"; screen: string; value: number; frames: number; tween: string | null }
  | { type: "remap"; screen: string }
  | { type: "template"; screen: string; template: string; cg_layer: number; data: string | null }
  | { type: "templatestop"; screen: string; cg_layer: number }
  | { type: "raw"; command: string };

export interface Cue {
  id: string;
  name: string;
  actions: Action[];
  follow: number | null;
  colour: string | null;
}

export interface Pad {
  index: number;
  cue: string;
}

export interface Show {
  name: string;
  canvas: { width: number; height: number };
  screens: Screen[];
  cues: Cue[];
  pads: Pad[];
  grid: [number, number];
}

export interface LogEntry {
  at: number;
  command: string;
  code: number | null;
  status: string;
  ok: boolean;
}

export interface ServerInfo {
  host: string;
  port: number;
  version: string | null;
  paths: string | null;
  osc_port: number | null;
}

export interface Snapshot {
  health: Health;
  server: ServerInfo;
  channels: ChannelState[];
  media: MediaItem[];
  templates: Template[];
  fonts: string[];
  scanner_up: boolean;
  show: Show;
  warnings: string[];
  log: LogEntry[];
}

/** The snapshot shown before the first one arrives. */
export const EMPTY: Snapshot = {
  health: "connecting",
  server: { host: "", port: 0, version: null, paths: null, osc_port: null },
  channels: [],
  media: [],
  templates: [],
  fonts: [],
  scanner_up: false,
  show: {
    name: "",
    canvas: { width: 1920, height: 1080 },
    screens: [],
    cues: [],
    pads: [],
    grid: [8, 4],
  },
  warnings: [],
  log: [],
};

/** Seconds as `m:ss.f`, the form an operator reads at a glance. */
export function timecode(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return "—";
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, "0")}`;
}

/** The media kind, derived the same way the daemon does. */
export function mediaKind(item: MediaItem): "movie" | "still" | "audio" | "unknown" {
  const streams = item.streams ?? [];
  const hasVideo = streams.some((s) => s.codec?.type === "video");
  const hasAudio = streams.some((s) => s.codec?.type === "audio");
  const duration = Number(item.format?.duration ?? 0);
  if (hasVideo && duration > 1) return "movie";
  if (hasVideo) return "still";
  if (hasAudio) return "audio";
  return "unknown";
}
