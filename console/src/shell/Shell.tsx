import type { ReactNode } from "react";
import type { ConnStatus } from "../lib/connection";
import type { Health, Snapshot } from "../lib/types";

export type PageId = "media" | "screens" | "channels" | "cues" | "templates" | "grid";

export interface PageDef {
  id: PageId;
  label: string;
  icon: string;
}

export const PAGES: PageDef[] = [
  { id: "media", label: "Media", icon: "▦" },
  { id: "screens", label: "Screens", icon: "◫" },
  { id: "channels", label: "Channels", icon: "▤" },
  { id: "cues", label: "Cues", icon: "▶" },
  { id: "templates", label: "Templates", icon: "T" },
  { id: "grid", label: "Grid", icon: "⊞" },
];

/** The shared page frame: top toolbar, left/centre/right, bottom. */
export function Frame(props: {
  toolbar?: ReactNode;
  left?: ReactNode;
  centre?: ReactNode;
  right?: ReactNode;
  bottom?: ReactNode;
}) {
  return (
    <div className="frame">
      {props.toolbar && <div className="frame-toolbar">{props.toolbar}</div>}
      <div className="frame-body">
        {props.left !== undefined && <aside className="frame-left">{props.left}</aside>}
        <section className="frame-centre">{props.centre}</section>
        {props.right !== undefined && <aside className="frame-right">{props.right}</aside>}
      </div>
      {props.bottom !== undefined && <div className="frame-bottom">{props.bottom}</div>}
    </div>
  );
}

/** A right-panel inspector with an honest empty state. */
export function Inspector(props: { title: string; children?: ReactNode; empty?: string }) {
  return (
    <div className="inspector">
      <div className="inspector-head">{props.title}</div>
      {props.children != null ? (
        <div className="inspector-body">{props.children}</div>
      ) : (
        <div className="inspector-empty">{props.empty ?? "Nothing to inspect"}</div>
      )}
    </div>
  );
}

/** A labelled read-only row in an inspector. */
export function Field(props: { label: string; children: ReactNode }) {
  return (
    <div className="field-row">
      <span className="field-label">{props.label}</span>
      <span className="field-value">{props.children}</span>
    </div>
  );
}

const STATUS_LABEL: Record<ConnStatus, string> = {
  connecting: "connecting…",
  live: "live",
  polling: "polling",
  down: "reconnecting…",
};

const SERVER_LABEL: Record<Health, string> = {
  connecting: "connecting to CasparCG…",
  connected: "CasparCG",
  down: "CasparCG offline",
};

export function TopBar(props: { status: ConnStatus; snapshot: Snapshot; right?: ReactNode }) {
  const { server, health } = props.snapshot;
  return (
    <header className="topbar">
      <span className="topbar-logo">◆</span>
      <span className="topbar-title">caspar-AV</span>
      <span className="topbar-show">{props.snapshot.show.name}</span>
      {props.right}
      <span className="spacer" />
      <span
        className={`server-state server-${health}`}
        title={
          server.version
            ? `${server.host}:${server.port} — ${server.version}`
            : `${server.host}:${server.port}`
        }
      >
        <span className="dot" />
        {SERVER_LABEL[health]}
        {server.version && <span className="dim small">{server.version.split(" ")[0]}</span>}
      </span>
      {/* Telemetry is a separate failure: commands can work perfectly while
          the OSC feed is missing, and the operator should be able to tell. */}
      {health === "connected" && server.osc_port == null && (
        <span className="chip warn" title="No OSC telemetry — positions and fps will not update">
          no telemetry
        </span>
      )}
      <span className={`conn conn-${props.status}`} title="Connection to caspar-avd">
        <span className="conn-dot" />
        {STATUS_LABEL[props.status]}
      </span>
      {/* Opens the shared About dialog — see console/public/about.js, which
          delegates this attribute from the document, so nothing is imported
          here. The version it shows is this console's, not the daemon's: the
          server's own version is already on the chip to the left. */}
      <button type="button" className="topbar-about" data-stoatworks-about>
        About
      </button>
    </header>
  );
}

export function PageTabs(props: { active: PageId; onSelect: (id: PageId) => void }) {
  return (
    <nav className="pagetabs">
      {PAGES.map((p) => (
        <button
          key={p.id}
          className={`pagetab ${p.id === props.active ? "active" : ""}`}
          onClick={() => props.onSelect(p.id)}
        >
          <span className="pagetab-icon">{p.icon}</span>
          <span className="pagetab-label">{p.label}</span>
        </button>
      ))}
    </nav>
  );
}

/** The command log, shared by every page's bottom dock. */
export function CommandLog(props: { snapshot: Snapshot; children?: ReactNode }) {
  return (
    <div className="panel">
      {props.children}
      <div className="log">
        {props.snapshot.log.length === 0 && <div className="list-empty">No commands sent yet</div>}
        {props.snapshot.log.map((entry, i) => (
          <div key={`${entry.at}-${i}`} className={`log-row ${entry.ok ? "" : "bad"}`}>
            <span className="log-code">{entry.code ?? "—"}</span>
            <span className="log-cmd">{entry.command}</span>
            <span className="log-status">{entry.status}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
