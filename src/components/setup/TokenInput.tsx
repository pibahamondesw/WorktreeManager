import { useState } from "react";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { SuccessCircleIcon, ExternalLinkIcon } from "../ui/Icons";

interface TokenInputProps {
  title: string;
  description: string;
  linkUrl: string;
  linkText: string;
  value: string;
  onChange: (value: string) => void;
  onValidate: (value: string) => Promise<{ valid: boolean; name?: string; error?: string }>;
  isValid: boolean;
  validatedName: string | null;
  placeholder: string;
}

export function TokenInput({
  title,
  description,
  linkUrl,
  linkText,
  value,
  onChange,
  onValidate,
  isValid,
  validatedName,
  placeholder,
}: TokenInputProps) {
  const [validating, setValidating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleValidate = async () => {
    if (!value.trim()) return;
    setValidating(true);
    setError(null);
    try {
      const result = await onValidate(value.trim());
      if (!result.valid) {
        setError(result.error ?? "Validation failed");
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Validation failed");
    } finally {
      setValidating(false);
    }
  };

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-base font-medium text-text-primary">{title}</h3>
        <p className="text-sm text-text-secondary mt-1">{description}</p>
      </div>

      <div className="flex items-end gap-2">
        <div className="flex-1">
          <Input
            value={value}
            onChange={(e) => {
              onChange(e.target.value);
              setError(null);
            }}
            placeholder={placeholder}
            type="password"
            error={error ?? undefined}
            onKeyDown={(e) => {
              if (e.key === "Enter") handleValidate();
            }}
          />
        </div>
        <Button
          variant="secondary"
          onClick={handleValidate}
          loading={validating}
          disabled={!value.trim()}
        >
          Validate
        </Button>
      </div>

      {isValid && validatedName && (
        <div className="flex items-center gap-2 text-sm text-success">
          <SuccessCircleIcon />
          Connected as {validatedName}
        </div>
      )}

      <button
        type="button"
        className="inline-flex items-center gap-1 text-sm text-accent hover:text-accent-hover transition-colors"
        onClick={async () => {
          const { openUrl } = await import("@tauri-apps/plugin-opener");
          openUrl(linkUrl);
        }}
      >
        {linkText}
        <ExternalLinkIcon size={12} />
      </button>
    </div>
  );
}
