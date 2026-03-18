import { splitProps, type JSX } from "solid-js";
import { cn } from "../../lib/cn";

export type SelectProps = JSX.SelectHTMLAttributes<HTMLSelectElement>;

export function Select(props: SelectProps) {
  const [local, rest] = splitProps(props, ["class"]);

  return (
    <select
      class={cn(
        "h-7 rounded border border-subtle bg-surface-1 px-2 text-xs text-primary transition-colors hover:border-default focus-visible:border-accent/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-subtle)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--surface-1)] motion-reduce:transition-none",
        local.class,
      )}
      {...rest}
    />
  );
}
