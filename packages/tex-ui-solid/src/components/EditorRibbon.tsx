import { For, Show } from "solid-js";

export type RibbonActionGroup = "Clipboard" | "Structure" | "Format" | "View";

export type RibbonAction = {
  key: string;
  label: string;
  group: RibbonActionGroup;
  active: boolean;
  run: () => void | Promise<void>;
};

type EditorRibbonProps = {
  actions: RibbonAction[];
  disabled: boolean;
  invisibilityMode: boolean;
  stickyHighlightMode: boolean;
  saving: boolean;
  ribbonOpen: boolean;
  onNewFile: () => void;
  onSave: () => void;
  onToggleInvisibilityMode: () => void;
  onToggleStickyHighlightMode: () => void;
  onToggleRibbon: () => void;
};

export default function EditorRibbon(props: EditorRibbonProps) {
  const preserveEditorSelection = (event: Event) => {
    event.preventDefault();
  };

  const runEditorActionFromPointer = (
    event: PointerEvent,
    action: () => void | Promise<void>,
  ) => {
    event.preventDefault();
    if (event.button !== 0) {
      return;
    }

    (event.currentTarget as HTMLButtonElement | null)?.blur();
    window.requestAnimationFrame(() => {
      void action();
    });
  };

  const runEditorActionFromClick = (
    event: MouseEvent,
    action: () => void | Promise<void>,
  ) => {
    if (event.detail !== 0) {
      event.preventDefault();
      (event.currentTarget as HTMLButtonElement | null)?.blur();
      return;
    }

    window.requestAnimationFrame(() => {
      void action();
    });
  };

  return (
    <div class="ribbon-shell">
      {/* Tab row — always visible */}
      <div class="ribbon-tab-row">
        <button
          class="ribbon-tab"
          classList={{ active: props.ribbonOpen }}
          onMouseDown={preserveEditorSelection}
          onPointerDown={preserveEditorSelection}
          onClick={props.onToggleRibbon}
          type="button"
        >
          Home
        </button>
      </div>

      {/* Panel — slides in/out */}
      <Show when={props.ribbonOpen}>
        <div class="ribbon-panel">
          <div class="ribbon-grid">
            <For each={props.actions.filter((a) => a.key !== "Cmd/Ctrl+Shift+I")}>
              {(action) => (
                <button
                  class="ribbon-btn"
                  classList={{ active: action.active }}
                  disabled={props.disabled}
                  onMouseDown={preserveEditorSelection}
                  onPointerDown={(event) => runEditorActionFromPointer(event, action.run)}
                  onClick={(event) => runEditorActionFromClick(event, action.run)}
                  title={`${action.key} - ${action.label}`}
                  type="button"
                >
                  <span class="ribbon-btn-key">{action.key}</span>
                  <span class="ribbon-btn-sep"> - </span>
                  <span class="ribbon-btn-label">{action.label}</span>
                </button>
              )}
            </For>
          </div>

          <div class="ribbon-panel-separator" />

          <button
            aria-label="Toggle sticky highlight mode"
            aria-pressed={props.stickyHighlightMode}
            class="ribbon-compact-toggle"
            classList={{ active: props.stickyHighlightMode }}
            disabled={props.disabled}
            onMouseDown={preserveEditorSelection}
            onPointerDown={preserveEditorSelection}
            onClick={props.onToggleStickyHighlightMode}
            title="Toggle sticky highlight mode"
            type="button"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M4 18l4-4" />
              <path d="M14 8l6-6" />
              <path d="M15 3h6v6" />
              <path d="M10 7l7 7" />
              <path d="M5 14l5 5" />
              <path d="M3 21h6" />
            </svg>
          </button>

          <button
            class="ribbon-compact-action"
            disabled={props.disabled}
            onMouseDown={preserveEditorSelection}
            onPointerDown={preserveEditorSelection}
            onClick={props.onNewFile}
            title="New file (Cmd/Ctrl+N)"
            type="button"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d="M12 5v14" />
              <path d="M5 12h14" />
            </svg>
            <span>New</span>
          </button>

          <button
            aria-label="Toggle invisibility mode"
            aria-pressed={props.invisibilityMode}
            class="ribbon-compact-toggle"
            classList={{ active: props.invisibilityMode }}
            disabled={props.disabled}
            onMouseDown={preserveEditorSelection}
            onPointerDown={preserveEditorSelection}
            onClick={props.onToggleInvisibilityMode}
            title="Toggle invisibility mode (Cmd/Ctrl+Shift+I)"
            type="button"
          >
            <svg
              width="16"
              height="16"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <Show
                when={!props.invisibilityMode}
                fallback={
                  <>
                    <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94" />
                    <path d="M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19" />
                    <line x1="1" y1="1" x2="23" y2="23" />
                  </>
                }
              >
                <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                <circle cx="12" cy="12" r="3" />
              </Show>
            </svg>
          </button>
        </div>
      </Show>
    </div>
  );
}
