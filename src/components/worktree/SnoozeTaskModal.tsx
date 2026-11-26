import { FormEvent, useState } from "react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Input } from "../ui/Input";
import { startOfLocalDay } from "../../utils";

function dateInputValue(date: Date) {
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${date.getFullYear()}-${month}-${day}`;
}

function startOfPickedDay(value: string) {
  const [year, month, day] = value.split("-").map(Number);
  return new Date(year, month - 1, day);
}

export function SnoozeTaskModal({
  onSnooze,
  onClose,
}: {
  onSnooze: (until: string) => Promise<void>;
  onClose: () => void;
}) {
  const [earliest] = useState(() => dateInputValue(startOfLocalDay(Date.now(), 1)));
  const [value, setValue] = useState(earliest);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const close = () => {
    if (!saving) onClose();
  };

  const snooze = async (event: FormEvent) => {
    event.preventDefault();
    const until = startOfPickedDay(value);
    if (saving || !value || until.getTime() <= Date.now()) return;
    setSaving(true);
    setError(null);
    try {
      await onSnooze(until.toISOString());
      onClose();
    } catch (error) {
      setError(String(error));
      setSaving(false);
    }
  };

  return (
    <Modal open onClose={close} title="Snooze task">
      <form onSubmit={(event) => void snooze(event)} className="flex flex-col gap-4">
        <Input
          type="date"
          label="Snooze until"
          min={earliest}
          value={value}
          onChange={(event) => setValue(event.target.value)}
          error={error ?? undefined}
          autoFocus
        />
        <div className="flex justify-end gap-3">
          <Button variant="ghost" type="button" onClick={close} disabled={saving}>
            Cancel
          </Button>
          <Button type="submit" disabled={saving || !value || value < earliest}>
            Snooze
          </Button>
        </div>
      </form>
    </Modal>
  );
}
