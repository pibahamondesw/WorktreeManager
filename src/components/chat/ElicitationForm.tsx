import { useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ChatResponse, PendingRequest } from "../../services/chat";
import { Button } from "../ui/Button";

type Request = Extract<PendingRequest, { kind: "elicitation" }>;
type Schema = Record<string, unknown>;
type Draft = string | string[];

const inputClass =
  "rounded-md bg-bg-secondary border border-border px-2 py-1 text-xs text-text-primary focus:outline-none focus:border-accent";

function schemaObject(value: unknown): Schema {
  return value !== null && typeof value === "object" && !Array.isArray(value)
    ? (value as Schema)
    : {};
}

function enumOptions(schema: Schema): { value: string; title: string }[] | null {
  if (Array.isArray(schema.enum))
    return schema.enum.map((value, i) => ({
      value: JSON.stringify(value),
      title: String((schema.enumNames as string[] | undefined)?.[i] ?? value),
    }));
  if (
    Array.isArray(schema.oneOf) &&
    schema.oneOf.every((option) => "const" in schemaObject(option))
  )
    return schema.oneOf.map((option: unknown) => {
      const item = schemaObject(option);
      return { value: JSON.stringify(item.const), title: String(item.title ?? item.const) };
    });
  return null;
}

function initialDraft(schema: Schema): Draft {
  if (schema.default === undefined) return "";
  if (schema.type === "array" && enumOptions(schemaObject(schema.items)))
    return Array.isArray(schema.default)
      ? schema.default.map((value) => JSON.stringify(value))
      : "";
  return schema.type === "string" && !enumOptions(schema)
    ? String(schema.default)
    : JSON.stringify(schema.default);
}

function fieldValue(schema: Schema, draft: Draft): unknown {
  if (Array.isArray(draft)) return draft.map((value) => JSON.parse(value) as unknown);
  if (schema.type === "string" && !enumOptions(schema)) return draft;
  if (schema.type === "number" || schema.type === "integer") {
    const number = Number(draft);
    if (!Number.isFinite(number)) throw new Error("Invalid number");
    return number;
  }
  return JSON.parse(draft) as unknown;
}

function SchemaField({
  name,
  schema,
  value,
  required,
  onChange,
}: {
  name: string;
  schema: Schema;
  value: Draft;
  required: boolean;
  onChange: (value: Draft) => void;
}) {
  const multiple = schema.type === "array";
  const options = enumOptions(multiple ? schemaObject(schema.items) : schema);
  const label = String(schema.title ?? name);
  const needsValue =
    required &&
    (schema.type === "string" && !options
      ? Number(schema.minLength ?? 0) > 0
      : multiple && options
        ? Number(schema.minItems ?? 0) > 0
        : true);
  const common = { "aria-label": label, required: needsValue, className: inputClass };
  return (
    <label className="flex flex-col gap-1 text-xs text-text-secondary select-text">
      <span>
        {label}
        {required ? " *" : ""}
      </span>
      {typeof schema.description === "string" && <span>{schema.description}</span>}
      {options ? (
        <select
          {...common}
          multiple={multiple}
          value={multiple ? (Array.isArray(value) ? value : []) : value}
          onChange={(e) =>
            onChange(
              multiple
                ? Array.from(e.target.selectedOptions, (option) => option.value)
                : e.target.value
            )
          }
        >
          {!multiple && <option value="">Select…</option>}
          {options.map((option) => (
            <option key={option.value} value={option.value}>
              {option.title}
            </option>
          ))}
        </select>
      ) : schema.type === "boolean" ? (
        <select {...common} value={value} onChange={(e) => onChange(e.target.value)}>
          <option value="">Select…</option>
          <option value="true">Yes</option>
          <option value="false">No</option>
        </select>
      ) : schema.type === "string" ? (
        <input
          {...common}
          type={
            schema.format === "email"
              ? "email"
              : schema.format === "uri"
                ? "url"
                : schema.format === "date"
                  ? "date"
                  : "text"
          }
          value={value}
          minLength={typeof schema.minLength === "number" ? schema.minLength : undefined}
          maxLength={typeof schema.maxLength === "number" ? schema.maxLength : undefined}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : schema.type === "number" || schema.type === "integer" ? (
        <input
          {...common}
          type="number"
          step={schema.type === "integer" ? 1 : "any"}
          min={typeof schema.minimum === "number" ? schema.minimum : undefined}
          max={typeof schema.maximum === "number" ? schema.maximum : undefined}
          value={value}
          onChange={(e) => onChange(e.target.value)}
        />
      ) : (
        <textarea
          {...common}
          value={value}
          rows={4}
          onChange={(e) => onChange(e.target.value)}
          placeholder="JSON value"
        />
      )}
    </label>
  );
}

export function ElicitationForm({
  request,
  disabled,
  onRespond,
}: {
  request: Request;
  disabled?: boolean;
  onRespond: (response: ChatResponse) => Promise<void>;
}) {
  const schema = request.mode === "form" ? schemaObject(request.requestedSchema) : {};
  const properties = schemaObject(schema.properties);
  const fields =
    schema.type === "object" && schema.properties !== undefined
      ? Object.entries(properties).map(([name, field]) => [name, schemaObject(field)] as const)
      : null;
  const [drafts, setDrafts] = useState<Record<string, Draft>>(() =>
    Object.fromEntries(fields?.map(([name, field]) => [name, initialDraft(field)]) ?? [])
  );
  const [json, setJson] = useState(() => JSON.stringify(schema.default ?? {}, null, 2));
  const [editingJson, setEditingJson] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [opening, setOpening] = useState(false);
  const responding = useRef(false);

  const collectFields = () =>
    Object.fromEntries(
      (fields ?? []).flatMap(([name, field]) => {
        const draft = drafts[name] ?? "";
        const required = Array.isArray(schema.required) && schema.required.includes(name);
        if (
          draft === "" &&
          field.type === "string" &&
          !enumOptions(field) &&
          (required || field.default === "")
        )
          return [[name, ""]];
        if (
          draft === "" &&
          required &&
          field.type === "array" &&
          enumOptions(schemaObject(field.items))
        )
          return [[name, []]];
        return draft === "" ? [] : [[name, fieldValue(field, draft)]];
      })
    );

  const editAsJson = () => {
    try {
      setJson(JSON.stringify(collectFields(), null, 2));
      setEditingJson(true);
      setError(null);
    } catch {
      setError("Enter a valid JSON value for the requested fields.");
    }
  };

  const respond = async (action: "accept" | "decline" | "cancel") => {
    if (responding.current || disabled) return;
    setError(null);
    let content: unknown = null;
    if (action === "accept" && request.mode === "form") {
      try {
        content = fields && !editingJson ? collectFields() : JSON.parse(json);
      } catch {
        setError("Enter a valid JSON value for the requested fields.");
        return;
      }
    }
    responding.current = true;
    setSending(true);
    try {
      await onRespond({ elicitation: { action, content } });
    } catch (e) {
      setError(String(e));
    } finally {
      responding.current = false;
      setSending(false);
    }
  };

  const open = async () => {
    if (request.mode !== "url") return;
    setError(null);
    setOpening(true);
    try {
      await openUrl(request.url);
    } catch {
      setError("Could not open the URL. Retry or cancel this request.");
    } finally {
      setOpening(false);
    }
  };

  return (
    <form
      className="flex flex-col gap-3"
      onSubmit={(e) => {
        e.preventDefault();
        if (e.currentTarget.reportValidity()) void respond("accept");
      }}
    >
      <fieldset disabled={sending || disabled} className="flex flex-col gap-3 min-w-0">
        <legend className="sr-only">Request from {request.serverName}</legend>
        {request.mode === "form" ? (
          <>
            {fields && !editingJson ? (
              <>
                <Button
                  type="button"
                  variant="secondary"
                  className="h-7 px-3 text-xs self-start"
                  onClick={editAsJson}
                >
                  Edit as JSON
                </Button>
                {fields.map(([name, field]) => (
                  <SchemaField
                    key={name}
                    name={name}
                    schema={field}
                    value={drafts[name] ?? ""}
                    required={Array.isArray(schema.required) && schema.required.includes(name)}
                    onChange={(value) => setDrafts((previous) => ({ ...previous, [name]: value }))}
                  />
                ))}
              </>
            ) : (
              <label className="flex flex-col gap-1 text-xs text-text-secondary">
                Form data (JSON)
                <textarea
                  className={inputClass}
                  value={json}
                  rows={6}
                  onChange={(e) => setJson(e.target.value)}
                />
              </label>
            )}
            <details className="text-xs text-text-secondary select-text">
              <summary className="cursor-pointer">Requested fields and constraints</summary>
              <pre className="whitespace-pre-wrap [overflow-wrap:anywhere] max-h-60 overflow-auto">
                {JSON.stringify(request.requestedSchema, null, 2)}
              </pre>
            </details>
          </>
        ) : (
          <>
            <p className="text-xs text-text-secondary select-text [overflow-wrap:anywhere]">
              {request.url}
            </p>
            <Button
              type="button"
              variant="secondary"
              disabled={opening}
              onClick={() => void open()}
            >
              Open URL
            </Button>
            <p className="text-xs text-text-secondary">
              Complete the flow in your browser, then confirm here. Opening the URL does not approve
              this request.
            </p>
          </>
        )}
        {error && (
          <p role="alert" className="text-xs text-danger select-text [overflow-wrap:anywhere]">
            {error}
          </p>
        )}
        <div className="flex flex-wrap gap-2">
          <Button type="submit" className="h-7 px-3 text-xs">
            {request.mode === "url" ? "Confirm completed" : "Accept"}
          </Button>
          <Button
            type="button"
            variant="secondary"
            className="h-7 px-3 text-xs"
            onClick={() => void respond("decline")}
          >
            Decline
          </Button>
          <Button
            type="button"
            variant="secondary"
            className="h-7 px-3 text-xs"
            onClick={() => void respond("cancel")}
          >
            Cancel
          </Button>
        </div>
      </fieldset>
    </form>
  );
}
