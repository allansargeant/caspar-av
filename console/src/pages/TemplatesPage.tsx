import { useEffect, useMemo, useState } from "react";
import * as actions from "../lib/actions";
import type { GddSchema } from "../lib/types";
import { CommandLog, Field, Frame, Inspector } from "../shell/Shell";
import { run, ScreenPicker, type PageProps } from "./common";

/**
 * Graphics templates.
 *
 * media-scanner extracts a **GDD** (Graphics Data Definition) from HTML
 * templates that publish one — a JSON Schema of the fields the template
 * expects. Where that exists this builds a real form; where it does not, it
 * falls back to a JSON box rather than pretending to know the fields.
 */
export function TemplatesPage(props: PageProps) {
  const { snapshot, target } = props;
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [cgLayer, setCgLayer] = useState(1);
  const [fields, setFields] = useState<Record<string, string>>({});
  const [rawJson, setRawJson] = useState("{}");

  const template = snapshot.templates.find((t) => t.id === selectedId) ?? null;
  const schema = template?.gdd ?? null;
  const properties = useMemo(() => gddProperties(schema), [schema]);

  // Reset the form when the template changes — carrying one template's fields
  // into another is never what was meant.
  useEffect(() => {
    setFields({});
    setRawJson("{}");
  }, [selectedId]);

  // Open on the first template, so the page shows a form rather than a prompt.
  useEffect(() => {
    const list = snapshot.templates;
    if (list.length > 0 && !list.some((t) => t.id === selectedId)) {
      setSelectedId(list[0].id);
    }
  }, [snapshot.templates, selectedId]);

  const data = properties
    ? JSON.stringify(Object.fromEntries(Object.entries(fields).filter(([, v]) => v !== "")))
    : rawJson;

  const send = (action: string) => {
    if (!target || !template) return;
    run(
      actions.template(target, {
        template: template.id,
        cg_layer: cgLayer,
        data,
        action,
      }),
    );
  };

  const toolbar = (
    <>
      <span className="toolbar-title">Templates</span>
      <span className="chip">{snapshot.templates.length}</span>
      <span className="spacer" />
      <ScreenPicker {...props} />
      <span className="small muted">CG layer</span>
      <input
        className="num"
        type="number"
        min={1}
        value={cgLayer}
        onChange={(e) => setCgLayer(Math.max(1, Number(e.target.value)))}
      />
    </>
  );

  const left = (
    <div className="panel">
      <div className="panel-head">Templates</div>
      <div className="list">
        {snapshot.templates.length === 0 && (
          <div className="list-empty">
            {snapshot.scanner_up ? "No templates found" : "media-scanner is not running"}
          </div>
        )}
        {snapshot.templates.map((t) => (
          <div
            key={t.id}
            className={`list-row ${t.id === selectedId ? "sel" : ""}`}
            onClick={() => setSelectedId(t.id)}
          >
            <span className="row-name mono small">{t.id}</span>
            {t.gdd && <span className="chip">GDD</span>}
            <span className="dim small">{t.type}</span>
          </div>
        ))}
      </div>
    </div>
  );

  const centre = (
    <div className="canvas-wrap">
      {!template ? (
        <div className="list-empty">Select a template</div>
      ) : (
        <div className="stack" style={{ maxWidth: 560, width: "100%", alignSelf: "center" }}>
          <div className="panel-head">{template.id}</div>
          {template.error && <div className="banner error small">GDD error: {template.error}</div>}

          {properties ? (
            <div className="gdd-form">
              {Object.entries(properties).map(([key, field]) => (
                <div className="gdd-field" key={key}>
                  <label className="gdd-label">{field.title ?? key}</label>
                  {field.enum ? (
                    <select
                      value={fields[key] ?? ""}
                      onChange={(e) => setFields({ ...fields, [key]: e.target.value })}
                    >
                      <option value="">—</option>
                      {field.enum.map((v) => (
                        <option key={v} value={v}>
                          {v}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      value={fields[key] ?? ""}
                      placeholder={String(field.default ?? "")}
                      onChange={(e) => setFields({ ...fields, [key]: e.target.value })}
                    />
                  )}
                  {field.description && <span className="gdd-desc">{field.description}</span>}
                </div>
              ))}
            </div>
          ) : (
            <div className="gdd-field">
              <label className="gdd-label">
                Template data (JSON) — this template publishes no GDD schema
              </label>
              <textarea value={rawJson} onChange={(e) => setRawJson(e.target.value)} spellCheck={false} />
            </div>
          )}

          <div className="inspector-actions">
            <button className="btn-primary" disabled={!target} onClick={() => send("add")}>
              Play
            </button>
            <button className="btn" disabled={!target} onClick={() => send("update")}>
              Update
            </button>
            <button className="btn" disabled={!target} onClick={() => send("next")}>
              Next
            </button>
            <button className="btn" disabled={!target} onClick={() => send("stop")}>
              Stop
            </button>
          </div>
          {!target && <span className="small dim">Add a screen to play a template onto.</span>}
        </div>
      )}
    </div>
  );

  const right = (
    <Inspector title="Template" empty="Select a template">
      {template ? (
        <>
          <Field label="Id">
            <span className="mono small">{template.id}</span>
          </Field>
          <Field label="Type">{template.type ?? "—"}</Field>
          <Field label="Schema">{template.gdd ? "GDD published" : "none"}</Field>
          <Field label="Fields">{properties ? Object.keys(properties).length : "—"}</Field>
          <div className="inspector-sub">Data being sent</div>
          <pre className="mono small" style={{ whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
            {data}
          </pre>
        </>
      ) : undefined}
    </Inspector>
  );

  return (
    <Frame toolbar={toolbar} left={left} centre={centre} right={right} bottom={<CommandLog snapshot={snapshot} />} />
  );
}

/**
 * The field map out of a GDD schema.
 *
 * GDD wraps its fields in a JSON Schema object; templates vary in whether the
 * useful properties sit at the root or under a `data` object, so both are
 * checked before giving up and falling back to the raw JSON editor.
 */
function gddProperties(schema: GddSchema | null): Record<string, GddSchema> | null {
  if (!schema) return null;
  const direct = schema.properties;
  if (direct && Object.keys(direct).length > 0) {
    const nested = direct.data?.properties;
    if (nested && Object.keys(nested).length > 0) return nested;
    return direct;
  }
  return null;
}
