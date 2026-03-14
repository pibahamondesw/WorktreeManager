import { ButtonHTMLAttributes, ReactNode } from "react";
import { SpinnerIcon } from "./Icons";

type Variant = "primary" | "secondary" | "danger" | "ghost";

const variants: Record<Variant, string> = {
  primary: "bg-accent text-white hover:bg-accent-hover disabled:opacity-40",
  secondary:
    "bg-bg-tertiary text-text-primary hover:bg-bg-hover border border-border disabled:opacity-40",
  danger: "bg-danger/10 text-danger hover:bg-danger/20 disabled:opacity-40",
  ghost:
    "bg-transparent text-text-secondary hover:bg-bg-hover hover:text-text-primary disabled:opacity-40",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children: ReactNode;
  loading?: boolean;
}

export function Button({
  variant = "primary",
  children,
  className = "",
  loading,
  disabled,
  ...props
}: ButtonProps) {
  return (
    <button
      className={`inline-flex items-center justify-center gap-2 rounded-lg px-4 py-2 text-sm font-medium transition-colors cursor-pointer ${variants[variant]} ${className}`}
      disabled={disabled || loading}
      {...props}
    >
      {loading && <SpinnerIcon size={16} />}
      {children}
    </button>
  );
}
