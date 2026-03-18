import { splitProps, type JSX } from "solid-js";
import { cn } from "../../lib/cn";

export type InputProps = JSX.InputHTMLAttributes<HTMLInputElement>;

export function Input(props: InputProps) {
  const [local, rest] = splitProps(props, ["class"]);

  return (
    <input
      class={cn(
        "h-7 w-full rounded border border-subtle bg-surface-1 px-2 text-xs text-primary transition-colors placeholder:text-ghost hover:border-default focus-visible:border-accent/50 focus-visible:bg-surface-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-subtle)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--surface-1)] motion-reduce:transition-none",
        local.class,
      )}
      {...rest}
    />
  );
}
