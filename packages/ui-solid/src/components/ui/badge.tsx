import { splitProps, type JSX } from "solid-js";
import { cn } from "../../lib/cn";

type BadgeVariant = "default" | "info" | "success" | "warning" | "muted";

const variantClasses: Record<BadgeVariant, string> = {
  default: "bg-surface-3 text-secondary border-subtle",
  info: "bg-[var(--blue-dim)] text-[var(--blue)] border-[rgba(96,165,250,0.2)]",
  success: "bg-[var(--accent-dim)] text-[var(--accent)] border-[rgba(45,212,191,0.2)]",
  warning: "bg-[var(--amber-dim)] text-[var(--amber)] border-[rgba(251,191,36,0.2)]",
  muted: "bg-surface-2 text-tertiary border-dim",
};

export type BadgeProps = JSX.HTMLAttributes<HTMLSpanElement> & {
  variant?: BadgeVariant;
};

export function Badge(props: BadgeProps) {
  const [local, rest] = splitProps(props, ["class", "variant"]);

  return (
    <span
      class={cn(
        "inline-flex items-center rounded border px-1.5 py-px text-2xs font-medium leading-tight",
        variantClasses[local.variant ?? "default"],
        local.class,
      )}
      {...rest}
    />
  );
}
