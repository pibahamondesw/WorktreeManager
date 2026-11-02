import { PermissionRequest } from "../../services/automations";

const inputText = (input: unknown) =>
  input === null || input === undefined
    ? "—"
    : typeof input === "string"
      ? input
      : JSON.stringify(input);

/** What the agent asked for that its unattended policy settled, to copy into the allowlist. */
export function AutomationPermissions({ requests }: { requests: PermissionRequest[] }) {
  return (
    <section aria-label="Permission requests" className="space-y-2">
      <h4 className="text-xs font-semibold text-text-primary">Permission requests</h4>
      <p className="text-xs text-text-muted">
        Add the ones this automation needs to{" "}
        <code className="font-mono">agent.permission.allow</code> in its AUTOMATION.md.
      </p>
      <table className="w-full text-xs select-text">
        <thead className="text-left text-text-muted">
          <tr>
            <th className="py-1.5 pr-4 font-medium">Step</th>
            <th className="py-1.5 pr-4 font-medium">Tool</th>
            <th className="py-1.5 pr-4 font-medium">Input</th>
            <th className="py-1.5 font-medium">Resolution</th>
          </tr>
        </thead>
        <tbody className="text-text-secondary">
          {requests.map((request, index) => (
            <tr key={index} className="border-t border-border align-top">
              <td className="py-1.5 pr-4 whitespace-nowrap">{request.step}</td>
              <td className="py-1.5 pr-4 font-mono">{request.tool}</td>
              <td className="py-1.5 pr-4 font-mono break-all">{inputText(request.input)}</td>
              <td className="py-1.5">{request.resolution}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}
