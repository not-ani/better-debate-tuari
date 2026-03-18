import { splitProps, type JSX } from "solid-js";
import { cn } from "../../lib/cn";

type ButtonVariant = "default" | "secondary" | "outline" | "ghost";
type ButtonSize = "default" | "sm" | "icon" | "icon-sm";

const variantClasses: Record<ButtonVariant, string> = {
  default:
    "bg-accent/90 text-surface-0 border-accent/60 hover:bg-accent hover:border-accent font-semibold disabled:opacity-40",
  secondary:
    "bg-surface-3 text-primary border-subtle hover:bg-surface-4 hover:text-primary hover:border-default disabled:opacity-40",
  outline:
    "bg-transparent text-secondary border-subtle hover:bg-surface-2 hover:text-primary hover:border-default disabled:opacity-40",
  ghost:
    "bg-transparent text-tertiary border-transparent hover:bg-surface-3 hover:text-secondary disabled:opacity-40",
};

const sizeClasses: Record<ButtonSize, string> = {
  default: "h-7 px-2.5 text-xs gap-1.5",
  sm: "h-6 px-2 text-2xs gap-1",
  icon: "h-7 w-7 px-0",
  "icon-sm": "h-6 w-6 px-0",
};

export type ButtonProps = JSX.ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: ButtonVariant;
  size?: ButtonSize;
};

export function Button(props: ButtonProps) {
  const [local, rest] = splitProps(props, ["class", "variant", "size"]);

  return (
    <button
      class={cn(
        "inline-flex items-center justify-center rounded border font-medium transition-colors duration-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--accent-subtle)] focus-visible:ring-offset-2 focus-visible:ring-offset-[var(--surface-1)] disabled:cursor-not-allowed motion-reduce:transition-none",
        variantClasses[local.variant ?? "secondary"],
        sizeClasses[local.size ?? "default"],
        local.class,
      )}
      {...rest}
    />
  );
}
