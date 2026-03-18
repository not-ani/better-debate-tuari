import { For, Show, type Accessor } from "solid-js";
import type { SidePreview } from "../lib/types";

type SidePreviewPaneProps = {
  sidePreview: Accessor<SidePreview | null>;
  width: Accessor<number>;
};

export default function SidePreviewPane(props: SidePreviewPaneProps) {
  return (
    <aside
      aria-label="Content preview"
      class="panel-surface flex min-h-0 shrink-0 flex-col"
      style={{
        width: `${props.width()}px`,
        "border-left": "1px solid var(--border-dim)",
      }}
    >
      {/* Header */}
      <div
        class="flex items-center justify-between px-2.5 py-1.5"
        style={{ "border-bottom": "1px solid var(--border-dim)", background: "var(--surface-2)" }}
      >
        <div class="flex items-center gap-1.5">
          <svg aria-hidden="true" class="h-3 w-3" style={{ color: "var(--accent)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M2.458 12C3.732 7.943 7.523 5 12 5c4.478 0 8.268 2.943 9.542 7-1.274 4.057-5.064 7-9.542 7-4.477 0-8.268-2.943-9.542-7z" />
          </svg>
          <h2 class="text-2xs font-medium" style={{ color: "var(--text-secondary)" }}>Preview</h2>
        </div>
        <p class="text-2xs" style={{ color: "var(--text-ghost)" }}>
          <span style={{ color: "var(--accent)" }}>Space</span> insert
          <span class="mx-1">&middot;</span>
          <span style={{ color: "var(--accent)" }}>C</span> copy
          <span class="mx-1">&middot;</span>
          <span style={{ color: "var(--accent)" }}>O</span> open
        </p>
      </div>

      {/* Content */}
      <div class="flex-1 overflow-hidden">
        <Show
          when={props.sidePreview()}
          fallback={
            <div class="flex h-full flex-col items-center justify-center p-4 text-center">
              <div
                aria-hidden="true"
                class="mb-2 flex h-10 w-10 items-center justify-center rounded-lg"
                style={{ background: "var(--surface-3)" }}
              >
                <svg class="h-5 w-5" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
                </svg>
              </div>
              <p class="text-xs font-medium" style={{ color: "var(--text-secondary)" }}>No content selected</p>
              <p class="mt-0.5 text-2xs" style={{ color: "var(--text-ghost)" }}>Hover over items in the tree to preview</p>
            </div>
          }
        >
          {(current) => (
            <div aria-live="polite" class="flex h-full flex-col">
              {/* Title bar */}
              <div
                class="px-2.5 py-2"
                style={{ "border-bottom": "1px solid var(--border-dim)", background: "var(--surface-0)" }}
              >
                <p
                  class={`text-xs font-medium ${current().kind === "heading" ? `preview-title-h${current().headingLevel ?? 0}` : ""}`}
                  style={{ color: "var(--text-primary)" }}
                >
                  {current().title}
                </p>
                <Show when={current().subTitle}>
                  <p class="mt-0.5 text-2xs" style={{ color: "var(--text-ghost)" }}>{current().subTitle}</p>
                </Show>
              </div>
              
              {/* Preview body */}
              <div class="flex-1 overflow-auto p-2.5">
                <Show
                  when={(current().richHtml?.trim().length ?? 0) > 0}
                  fallback={
                    <div class="preview-content text-[13px] leading-7">
                      <For each={current().text.split(/\n\s*\n/g).filter((part) => part.trim().length > 0)}>
                        {(paragraph) => (
                          <p class="mb-3 whitespace-pre-wrap">{paragraph}</p>
                        )}
                      </For>
                    </div>
                  }
                >
                  <div class="preview-rich" innerHTML={current().richHtml ?? ""} />
                </Show>
              </div>
            </div>
          )}
        </Show>
      </div>
    </aside>
  );
}
