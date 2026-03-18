import { Show, type Accessor } from "solid-js";
import type { IndexProgress } from "../lib/types";
import {
  formatElapsed,
  formatRate,
  getEstimatedTimeRemaining,
  getIndexProgressDetail,
  getIndexProgressPercent,
  getIndexProgressStageLabel,
  getIndexProgressTitle,
} from "../lib/indexProgress";
import { Button } from "./ui/button";

type IndexProgressPanelProps = {
  progress: Accessor<IndexProgress | null>;
  openPath: (path: string) => Promise<void>;
  includeDocxLabel?: boolean;
  showLogPath?: boolean;
  class?: string;
};

export default function IndexProgressPanel(props: IndexProgressPanelProps) {
  const percent = () => getIndexProgressPercent(props.progress());
  const eta = () => getEstimatedTimeRemaining(props.progress());
  const title = () => getIndexProgressTitle(props.progress(), { includeDocxLabel: props.includeDocxLabel });
  const detail = () => getIndexProgressDetail(props.progress());
  const stageLabel = () => getIndexProgressStageLabel(props.progress());

  return (
    <div
      aria-atomic="true"
      aria-live="polite"
      class={props.class}
      style={{ border: "1px solid var(--accent-subtle)", background: "var(--accent-dim)" }}
    >
      <div class="flex items-center justify-between gap-3 text-2xs">
        <div class="flex min-w-0 items-center gap-2">
          <div aria-hidden="true" class="h-2.5 w-2.5 animate-pulse rounded-full motion-reduce:animate-none" style={{ background: "var(--accent)" }} />
          <span class="truncate text-pretty" style={{ color: "var(--accent-bright)" }}>
            {title()}
          </span>
        </div>
        <div class="flex items-center gap-2">
          <span class="metric-chip" style={{ color: "var(--accent-bright)" }}>{stageLabel()}</span>
          <Show when={props.progress()?.phase === "indexing" && props.progress()?.changed !== 0}>
            <span class="metric-chip" style={{ color: "var(--accent)" }}>{percent()}%</span>
          </Show>
          <Show when={props.progress()?.logPath}>
            <Button
              onClick={() => void props.openPath(props.progress()!.logPath!)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Open Log
            </Button>
          </Show>
        </div>
      </div>

      <div class="mt-2 h-1.5 overflow-hidden rounded-full" style={{ background: "var(--surface-4)" }}>
        <Show
          when={props.progress()?.phase === "indexing" && props.progress()?.changed !== 0}
          fallback={<div class="h-full w-1/3 animate-pulse rounded-full motion-reduce:animate-none" style={{ background: "var(--accent-fg)" }} />}
        >
          <div
            class="h-full rounded-full transition-[width] duration-200 motion-reduce:transition-none"
            style={{ width: `${percent()}%`, background: "var(--accent)" }}
          />
        </Show>
      </div>

      <p class="mt-2 truncate text-2xs" style={{ color: "var(--accent-bright)" }}>
        {detail()}
      </p>

      <div class="mt-2 flex flex-wrap items-center gap-1 text-2xs" style={{ color: "var(--accent-bright)" }}>
        <span class="metric-chip">Scan {formatRate(props.progress()?.scanRatePerSec ?? 0)}</span>
        <span class="metric-chip">Process {formatRate(props.progress()?.processRatePerSec ?? 0)}</span>
        <span class="metric-chip">Phase {formatElapsed(props.progress()?.phaseElapsedMs ?? 0)}</span>
        <Show when={props.progress()?.phase === "indexing" && eta()}>
          <span class="metric-chip">ETA {eta()}</span>
        </Show>
      </div>

      <Show when={props.showLogPath && props.progress()?.logPath}>
        <p class="mt-2 truncate text-2xs" style={{ color: "var(--text-ghost)" }}>
          {props.progress()?.logPath}
        </p>
      </Show>
    </div>
  );
}
