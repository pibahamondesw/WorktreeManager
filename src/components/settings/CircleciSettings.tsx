import { FormEvent, useEffect, useState } from "react";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { circleciTokenSaved, saveCircleciToken } from "../../services/circleci";

export function CircleciSettings() {
  const [saved, setSaved] = useState<boolean | null>(null);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    circleciTokenSaved().then(setSaved, (reason) => setError(String(reason)));
  }, []);

  async function store(value: string | null) {
    setBusy(true);
    setError(null);
    try {
      await saveCircleciToken(value);
      setSaved(value !== null);
      setToken("");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  }

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    void store(token);
  }

  return (
    <form onSubmit={handleSubmit} className="px-6 pt-3 pb-6 space-y-3">
      <p className="text-xs text-text-muted">
        Automations with CircleCI event triggers poll CircleCI with your personal API token. It is
        stored in Keychain and never shown again.
      </p>
      <Input
        type="password"
        label="Personal API token"
        autoComplete="off"
        value={token}
        onChange={(event) => setToken(event.target.value)}
        placeholder={saved ? "Saved; enter a new token to replace it" : "No token saved"}
      />
      {error && <p className="text-xs text-danger select-text">{error}</p>}
      <div className="flex gap-2">
        <Button type="submit" disabled={busy || !token.trim()}>
          Save token
        </Button>
        {saved && (
          <Button type="button" variant="danger" disabled={busy} onClick={() => void store(null)}>
            Remove token
          </Button>
        )}
      </div>
    </form>
  );
}
