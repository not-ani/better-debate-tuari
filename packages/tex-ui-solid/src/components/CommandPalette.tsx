import { For, Show, createEffect, createSignal } from "solid-js";

export type CommandPaletteItem = {
  id: string;
  title: string;
  subtitle?: string;
  badge: string;
  meta?: string;
  run: () => void | Promise<void>;
};

type CommandPaletteProps = {
  open: boolean;
  query: string;
  items: CommandPaletteItem[];
  onClose: () => void;
  onExecute: (item: CommandPaletteItem) => void | Promise<void>;
  onQueryChange: (value: string) => void;
};

export default function CommandPalette(props: CommandPaletteProps) {
  let inputRef!: HTMLInputElement;
  const [selectedIndex, setSelectedIndex] = createSignal(0);

  createEffect(() => {
    if (!props.open) {
      return;
    }

    setSelectedIndex(0);
    queueMicrotask(() => inputRef?.focus());
  });

  createEffect(() => {
    const maxIndex = Math.max(props.items.length - 1, 0);
    setSelectedIndex((current) => Math.min(current, maxIndex));
  });

  const runItem = async (index: number) => {
    const item = props.items[index];
    if (!item) {
      return;
    }

    await props.onExecute(item);
  };

  const onInputKeyDown = (event: KeyboardEvent) => {
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex((current) => (props.items.length === 0 ? 0 : (current + 1) % props.items.length));
      return;
    }

    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex((current) =>
        props.items.length === 0 ? 0 : (current - 1 + props.items.length) % props.items.length,
      );
      return;
    }

    if (event.key === "Enter") {
      event.preventDefault();
      void runItem(selectedIndex());
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      props.onClose();
    }
  };

  return (
    <Show when={props.open}>
      <div class="command-palette-backdrop" onMouseDown={props.onClose}>
        <section
          aria-label="Command palette"
          class="command-palette"
          onMouseDown={(event) => event.stopPropagation()}
          role="dialog"
        >
          <header class="command-palette-header">
            <input
              autocomplete="off"
              class="command-palette-input"
              onInput={(event) => props.onQueryChange(event.currentTarget.value)}
              onKeyDown={onInputKeyDown}
              placeholder="Search files, windows, and commands"
              ref={inputRef}
              type="text"
              value={props.query}
            />
            <span class="command-palette-hint">Esc</span>
          </header>

          <div class="command-palette-results">
            <Show
              when={props.items.length > 0}
              fallback={
                <div class="command-palette-empty">
                  <p>No files or commands matched.</p>
                </div>
              }
            >
              <For each={props.items}>
                {(item, index) => (
                  <button
                    class="command-palette-item"
                    classList={{ selected: index() === selectedIndex() }}
                    onClick={() => void runItem(index())}
                    onMouseEnter={() => setSelectedIndex(index())}
                    type="button"
                  >
                    <span class="command-palette-badge">{item.badge}</span>

                    <span class="command-palette-copy">
                      <span class="command-palette-title">{item.title}</span>
                      <Show when={item.subtitle}>
                        <span class="command-palette-subtitle">{item.subtitle}</span>
                      </Show>
                    </span>

                    <Show when={item.meta}>
                      <span class="command-palette-meta">{item.meta}</span>
                    </Show>
                  </button>
                )}
              </For>
            </Show>
          </div>
        </section>
      </div>
    </Show>
  );
}
