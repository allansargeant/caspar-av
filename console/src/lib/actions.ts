// Every change the console can make, in one place.
//
// Each of these is fire-and-forget from the UI's point of view: the daemon
// applies it and the next snapshot reflects it. Nothing here updates local
// state optimistically, which is what keeps two consoles agreeing.

import { api } from "./connection";
import type { Cue, Pad, Screen, Show } from "./types";

// ---------------------------------------------------------------- transport

export const play = (screen: string, clip: string, looping = false, frames = 0) =>
  api("POST", `/screens/${encodeURIComponent(screen)}/transport`, {
    action: "play",
    clip,
    looping,
    frames,
  });

export const load = (screen: string, clip: string, looping = false, frames = 0) =>
  api("POST", `/screens/${encodeURIComponent(screen)}/transport`, {
    action: "load",
    clip,
    looping,
    frames,
  });

const simple = (action: string) => (screen: string) =>
  api("POST", `/screens/${encodeURIComponent(screen)}/transport`, { action });

export const take = simple("take");
export const pause = simple("pause");
export const resume = simple("resume");
export const stop = simple("stop");
export const clear = simple("clear");

// -------------------------------------------------------------------- mixer

export const mixer = (
  screen: string,
  property: string,
  values: number[],
  opts: { frames?: number; tween?: string | null; text?: string } = {},
) =>
  api("POST", `/screens/${encodeURIComponent(screen)}/mixer`, {
    property,
    values,
    frames: opts.frames ?? 0,
    tween: opts.tween ?? null,
    text: opts.text ?? null,
  });

// ------------------------------------------------------------------ screens

export const addScreen = (screen: Screen) => api("POST", "/screens", screen);
export const updateScreen = (screen: Screen) =>
  api("PATCH", `/screens/${encodeURIComponent(screen.id)}`, screen);
export const deleteScreen = (id: string) => api("DELETE", `/screens/${encodeURIComponent(id)}`);
export const pushMapping = () => api("POST", "/mapping/push");

// --------------------------------------------------------------------- cues

export const addCue = (cue: Cue) => api("POST", "/cues", cue);
export const updateCue = (cue: Cue) => api("PATCH", `/cues/${encodeURIComponent(cue.id)}`, cue);
export const deleteCue = (id: string) => api("DELETE", `/cues/${encodeURIComponent(id)}`);
export const fireCue = (id: string) => api("POST", `/cues/${encodeURIComponent(id)}/fire`);
export const setPads = (pads: Pad[]) => api("PUT", "/pads", pads);

// ---------------------------------------------------------------- templates

export const template = (
  screen: string,
  body: { template: string; cg_layer?: number; data?: string | null; action?: string; method?: string },
) => api("POST", `/screens/${encodeURIComponent(screen)}/template`, body);

// ---------------------------------------------------------------------- misc

export const setShow = (show: Show) => api("PUT", "/show", show);
export const refreshLibrary = () => api("POST", "/library/refresh");

export interface CommandResult {
  code: number;
  status: string;
  lines: string[];
}

/** Send a raw AMCP command. The escape hatch, and the debugging tool. */
export const rawCommand = (command: string) =>
  api<CommandResult>("POST", "/command", { command });

/** The URL of a media item's thumbnail, proxied through the daemon. */
export const thumbnailUrl = (id: string) => `api/media/${encodeURIComponent(id)}/thumbnail`;

/** A short, stable id from a name — good enough for screens and cues. */
export function slug(name: string, existing: string[]): string {
  const base =
    name
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "") || "item";
  if (!existing.includes(base)) return base;
  for (let n = 2; ; n++) {
    const candidate = `${base}-${n}`;
    if (!existing.includes(candidate)) return candidate;
  }
}
