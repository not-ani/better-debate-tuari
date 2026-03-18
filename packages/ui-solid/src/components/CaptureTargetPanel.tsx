import { DragDropProvider, useDraggable, useDroppable } from "@dnd-kit/solid";
import {
  For,
  Show,
  createMemo,
  createSignal,
  type Accessor,
  type Setter,
} from "solid-js";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Select } from "./ui/select";
import type {
  CaptureTarget,
  CaptureTargetPreview,
  FileHeading,
} from "../lib/types";

type CaptureTargetPanelProps = {
  captureRootPath: Accessor<string>;
  isLoadingCaptureTargets: Accessor<boolean>;
  selectedCaptureTarget: Accessor<string>;
  setSelectedCaptureTarget: (value: string) => void;
  captureTargets: Accessor<CaptureTarget[]>;
  createCaptureTarget: () => Promise<void>;
  selectCaptureTargetFromFilesystem: () => Promise<void>;
  isAllRootsSelected: Accessor<boolean>;
  selectedCaptureTargetMeta: Accessor<CaptureTarget | null>;
  isLoadingCapturePreview: Accessor<boolean>;
  captureTargetPreview: Accessor<CaptureTargetPreview | null>;
  captureTargetH1ToH4: Accessor<FileHeading[]>;
  selectedCaptureHeadingOrder: Accessor<number | null>;
  setSelectedCaptureHeadingOrder: Setter<number | null>;
  addCaptureHeading: (headingLevel: 1 | 2 | 3 | 4, headingText: string) => Promise<boolean>;
  deleteCaptureHeading: (headingOrder: number) => Promise<void>;
  moveCaptureHeading: (
    sourceHeadingOrder: number,
    targetHeadingOrder: number,
  ) => Promise<void>;
};

const DND_HEADING_PREFIX = "capture-preview-heading:";

const headingDndId = (headingOrder: number) =>
  `${DND_HEADING_PREFIX}${headingOrder}`;

const parseHeadingOrderFromDnd = (
  value: string | number | undefined | null,
) => {
  if (typeof value === "number") return Number.isFinite(value) ? value : null;
  if (typeof value !== "string" || !value.startsWith(DND_HEADING_PREFIX)) {
    return null;
  }
  const rawOrder = Number.parseInt(value.slice(DND_HEADING_PREFIX.length), 10);
  return Number.isFinite(rawOrder) ? rawOrder : null;
};

type PreviewHeadingRowProps = {
  heading: FileHeading;
  isSelected: boolean;
  isBusy: boolean;
  canMoveUp: boolean;
  canMoveDown: boolean;
  setSelectedCaptureHeadingOrder: Setter<number | null>;
  moveHeadingUp: (headingOrder: number) => Promise<void>;
  moveHeadingDown: (headingOrder: number) => Promise<void>;
  deleteCaptureHeading: (headingOrder: number) => Promise<void>;
};

function PreviewHeadingRow(props: PreviewHeadingRowProps) {
  const {
    ref: draggableRef,
    handleRef,
    isDragging,
  } = useDraggable({
    get id() {
      return headingDndId(props.heading.order);
    },
  });
  const { ref: droppableRef, isDropTarget } = useDroppable({
    get id() {
      return headingDndId(props.heading.order);
    },
  });

  const setCombinedRef = (element: Element | undefined) => {
    draggableRef(element);
    droppableRef(element);
  };

  return (
    <div
      class={`group flex items-center gap-1.5 rounded px-1.5 py-1 transition-colors transition-transform motion-reduce:transition-none ${isDragging() ? "opacity-50" : ""}`}
      role="listitem"
      style={{
        border: `1px solid ${
          isDropTarget()
            ? "var(--accent)"
            : props.isSelected
              ? "var(--accent-fg)"
              : "transparent"
        }`,
        background: isDropTarget()
          ? "var(--accent-dim)"
          : props.isSelected
            ? "var(--accent-dim)"
            : "transparent",
      }}
      ref={setCombinedRef}
    >
      <button
        aria-label={`Drag ${props.heading.text}`}
        class="cursor-grab rounded p-0.5 transition active:cursor-grabbing"
        style={{ color: "var(--text-ghost)" }}
        disabled={props.isBusy}
        ref={handleRef}
        type="button"
      >
        <svg
          class="h-2.5 w-2.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M4 8h16M4 16h16"
          />
        </svg>
      </button>
      <button
        class="min-w-0 flex-1 rounded py-0.5 text-left"
        onClick={() =>
          props.setSelectedCaptureHeadingOrder(props.heading.order)
        }
        style={{
          "padding-left": `${Math.max(0, props.heading.level - 1) * 10}px`,
        }}
        type="button"
      >
        <span
          class="mr-1 inline-flex rounded px-1 py-0 text-[8px] font-semibold uppercase"
          style={{
            background:
              props.heading.level === 1
                ? "var(--blue-dim)"
                : props.heading.level === 2
                  ? "var(--accent-dim)"
                  : "var(--surface-3)",
            color:
              props.heading.level === 1
                ? "var(--blue)"
                : props.heading.level === 2
                  ? "var(--accent)"
                  : "var(--text-tertiary)",
          }}
        >
          H{props.heading.level}
        </span>
        <span class="truncate text-2xs" style={{ color: "var(--text-primary)" }}>
          {props.heading.text}
        </span>
      </button>
      <div class="flex items-center gap-0.5">
        <button
          class="rounded p-0.5 transition hover:bg-[var(--surface-3)] disabled:opacity-30"
          style={{ color: "var(--text-ghost)" }}
          disabled={props.isBusy || !props.canMoveUp}
          onClick={(event) => {
            event.stopPropagation();
            void props.moveHeadingUp(props.heading.order);
          }}
          title="Move up"
          type="button"
        >
          <svg class="h-2.5 w-2.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 15l7-7 7 7" />
          </svg>
        </button>
        <button
          class="rounded p-0.5 transition hover:bg-[var(--surface-3)] disabled:opacity-30"
          style={{ color: "var(--text-ghost)" }}
          disabled={props.isBusy || !props.canMoveDown}
          onClick={(event) => {
            event.stopPropagation();
            void props.moveHeadingDown(props.heading.order);
          }}
          title="Move down"
          type="button"
        >
          <svg class="h-2.5 w-2.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M19 9l-7 7-7-7" />
          </svg>
        </button>
      </div>
      <button
        class="rounded p-0.5 transition hover:bg-[var(--rose-dim)] disabled:opacity-30"
        style={{ color: "var(--text-ghost)" }}
        disabled={props.isBusy}
        onClick={(event) => {
          event.stopPropagation();
          void props.deleteCaptureHeading(props.heading.order);
        }}
        type="button"
      >
        <svg
          class="h-2.5 w-2.5"
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            stroke-linecap="round"
            stroke-linejoin="round"
            stroke-width="2"
            d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
          />
        </svg>
      </button>
    </div>
  );
}

export default function CaptureTargetPanel(props: CaptureTargetPanelProps) {
  const quickHeadingActions = [
    { level: 1 as const, name: "pocket" },
    { level: 2 as const, name: "hat" },
    { level: 3 as const, name: "block" },
    { level: 4 as const, name: "section" },
  ];
  const headingNameByLevel: Record<1 | 2 | 3 | 4, string> = {
    1: "pocket",
    2: "hat",
    3: "block",
    4: "section",
  };
  const [newHeadingLevel, setNewHeadingLevel] = createSignal<1 | 2 | 3 | 4>(1);
  const [newHeadingName, setNewHeadingName] = createSignal(headingNameByLevel[1]);

  const setHeadingLevel = (level: 1 | 2 | 3 | 4) => {
    setNewHeadingLevel(level);
    setNewHeadingName(headingNameByLevel[level]);
  };

  const submitNewHeading = async () => {
    const created = await props.addCaptureHeading(newHeadingLevel(), newHeadingName());
    if (created) {
      setNewHeadingName("");
    }
  };

  const moveHintsByOrder = createMemo(() => {
    const headings = [...props.captureTargetH1ToH4()].sort(
      (left, right) => left.order - right.order,
    );
    const levelByOrder = new Map<number, number>();
    headings.forEach((heading) => {
      levelByOrder.set(heading.order, heading.level);
    });

    const siblingBuckets = new Map<string, number[]>();
    const stack: number[] = [];
    for (const heading of headings) {
      while (stack.length > 0) {
        const top = stack[stack.length - 1];
        const topLevel = levelByOrder.get(top) ?? 0;
        if (topLevel >= heading.level) {
          stack.pop();
          continue;
        }
        break;
      }

      const parentOrder = stack.length > 0 ? stack[stack.length - 1] : 0;
      const bucketKey = `${parentOrder}:${heading.level}`;
      const bucket = siblingBuckets.get(bucketKey) ?? [];
      bucket.push(heading.order);
      siblingBuckets.set(bucketKey, bucket);
      stack.push(heading.order);
    }

    const hints = new Map<number, { up: number | null; down: number | null }>();
    siblingBuckets.forEach((orders) => {
      orders.forEach((order, index) => {
        hints.set(order, {
          up: index > 0 ? orders[index - 1] : null,
          down: index < orders.length - 1 ? orders[index + 1] : null,
        });
      });
    });

    return hints;
  });

  const moveHeadingUp = async (headingOrder: number) => {
    const targetOrder = moveHintsByOrder().get(headingOrder)?.up;
    if (!targetOrder) return;
    await props.moveCaptureHeading(targetOrder, headingOrder);
  };

  const moveHeadingDown = async (headingOrder: number) => {
    const targetOrder = moveHintsByOrder().get(headingOrder)?.down;
    if (!targetOrder) return;
    await props.moveCaptureHeading(headingOrder, targetOrder);
  };

  const moveHintForHeading = (headingOrder: number) =>
    moveHintsByOrder().get(headingOrder);

  const selectedHeading = () =>
    props
      .captureTargetH1ToH4()
      .find(
        (heading) => heading.order === props.selectedCaptureHeadingOrder(),
      ) ?? null;
  const selectedTargetInfo = () => {
    const absolutePath = props.selectedCaptureTargetMeta()?.absolutePath;
    if (!absolutePath) return null;

    const normalized = absolutePath.replace(/\\/g, "/");
    const split = normalized.lastIndexOf("/");
    const fileName = split >= 0 ? normalized.slice(split + 1) : normalized;
    const parentPath = split > 0 ? normalized.slice(0, split) : "/";

    return { absolutePath, fileName, parentPath };
  };

  return (
    <div class="flex h-full min-h-0 flex-col">
      {/* Target selector bar */}
      <div
        class="flex items-center gap-1.5 px-2 py-1.5"
        style={{ "border-bottom": "1px solid var(--border-dim)", background: "var(--surface-2)" }}
      >
        <div class="flex flex-1 items-center gap-1 overflow-x-scroll">
          <label class="sr-only" for="capture-target-select">Capture target document</label>
          <Select
            aria-label="Capture target document"
            class="h-6 flex-1 px-1.5 text-2xs"
            disabled={
              !props.captureRootPath() || props.isLoadingCaptureTargets()
            }
            id="capture-target-select"
            onChange={(event) =>
              props.setSelectedCaptureTarget(event.currentTarget.value)
            }
            value={props.selectedCaptureTarget()}
          >
            <option value="" disabled>
              {props.isLoadingCaptureTargets()
                ? "Loading..."
                : "Select target .docx"}
            </option>
            <For each={props.captureTargets()}>
              {(target) => (
                <option value={target.relativePath}>
                  {target.relativePath} {!target.exists && "(new)"}
                </option>
              )}
            </For>
          </Select>
          <Button
            aria-label="Create capture target"
            disabled={!props.captureRootPath()}
            onClick={() => void props.createCaptureTarget()}
            size="icon-sm"
            title="Create new target"
            type="button"
            variant="secondary"
          >
            <svg
              class="h-3 w-3"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M12 4v16m8-8H4"
              />
            </svg>
          </Button>
          <Button
            aria-label="Browse for capture target"
            disabled={!props.captureRootPath()}
            onClick={() => void props.selectCaptureTargetFromFilesystem()}
            size="icon-sm"
            title="Browse for target"
            type="button"
            variant="secondary"
          >
            <svg
              class="h-3 w-3"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path
                stroke-linecap="round"
                stroke-linejoin="round"
                stroke-width="2"
                d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z"
              />
            </svg>
          </Button>
        </div>
      </div>

      {/* All-roots warning */}
      <Show when={props.isAllRootsSelected()}>
        <div
          class="flex items-center gap-1.5 px-2 py-1.5 text-2xs"
          style={{ "border-bottom": "1px solid var(--border-dim)", background: "var(--amber-dim)", color: "var(--amber)" }}
        >
          <svg
            class="h-3 w-3 shrink-0"
            fill="none"
            stroke="currentColor"
            viewBox="0 0 24 24"
          >
            <path
              stroke-linecap="round"
              stroke-linejoin="round"
              stroke-width="2"
              d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z"
            />
          </svg>
          <span>Select an individual root to manage destination files</span>
        </div>
      </Show>

      {/* Body */}
      <div class="flex min-h-0 flex-1 flex-col p-2">
        {/* Current target info */}
        <div class="flex items-start justify-between gap-2">
          <div class="min-w-0 flex-1">
            <p class="section-label">
              Current target
            </p>
            <Show
              when={selectedTargetInfo()}
              fallback={
                <p class="mt-0.5 truncate text-2xs" style={{ color: "var(--text-ghost)" }}>
                  No target selected
                </p>
              }
            >
              {(info) => (
                <>
                  <p
                    class="mt-0.5 truncate text-xs font-medium"
                    style={{ color: "var(--text-primary)" }}
                    title={info().absolutePath}
                  >
                    {info().fileName}
                  </p>
                  <p
                    class="truncate text-2xs"
                    style={{ color: "var(--text-ghost)" }}
                    title={info().absolutePath}
                  >
                    {info().parentPath}
                  </p>
                </>
              )}
            </Show>
          </div>

          <Badge class="shrink-0" variant="muted">
            {props.isLoadingCapturePreview() ? (
              <span class="flex items-center gap-1">
                <div
                  class="h-2.5 w-2.5 animate-spin rounded-full border"
                  style={{ "border-color": "var(--surface-4)", "border-top-color": "var(--accent)" }}
                />
              </span>
            ) : (
              `${props.captureTargetPreview()?.headingCount ?? 0} headings`
            )}
          </Badge>
        </div>

        {/* Selected heading indicator */}
        <Show when={selectedHeading()}>
          {(heading) => (
            <div
              class="mt-1.5 flex items-center justify-between gap-2 rounded px-2 py-1"
              style={{ border: "1px solid var(--accent-subtle)", background: "var(--accent-dim)" }}
            >
              <span class="truncate text-2xs" style={{ color: "var(--accent-bright)" }}>
                H{heading().level}: {heading().text}
              </span>
              <button
                class="rounded px-1 py-0.5 text-2xs transition hover:bg-[var(--surface-3)]"
                style={{ color: "var(--text-tertiary)" }}
                onClick={() => props.setSelectedCaptureHeadingOrder(null)}
                type="button"
              >
                Clear
              </button>
            </div>
          )}
        </Show>

        {/* Add heading section */}
        <div
          class="mt-1.5 rounded px-2 py-1.5"
          style={{ border: "1px solid var(--border-subtle)", background: "var(--surface-2)" }}
        >
          <div class="flex items-center justify-between gap-2">
            <p class="section-label">Add heading</p>
          </div>
          <div class="mt-1.5 flex flex-wrap gap-1">
            <For each={quickHeadingActions}>
              {(action) => (
                <Button
                  class="h-5 px-1.5 text-2xs"
                  disabled={!props.captureRootPath() || !props.selectedCaptureTarget() || props.isLoadingCapturePreview()}
                  onClick={() => void props.addCaptureHeading(action.level, action.name)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  <span
                    class="rounded px-0.5 py-0 text-[8px] font-semibold uppercase"
                    style={{ background: "var(--blue-dim)", color: "var(--blue)" }}
                  >
                    H{action.level}
                  </span>
                  <span>{action.name}</span>
                </Button>
              )}
            </For>
          </div>

          <div class="mt-1.5 flex items-center gap-1">
            <Select
              aria-label="Heading level"
              class="h-6 px-1.5 text-2xs"
              disabled={!props.captureRootPath() || !props.selectedCaptureTarget() || props.isLoadingCapturePreview()}
              onChange={(event) => {
                const level = Number.parseInt(event.currentTarget.value, 10);
                if (level >= 1 && level <= 4) {
                  setHeadingLevel(level as 1 | 2 | 3 | 4);
                }
              }}
              value={newHeadingLevel()}
            >
              <option value="1">H1</option>
              <option value="2">H2</option>
              <option value="3">H3</option>
              <option value="4">H4</option>
            </Select>

            <Input
              aria-label="Heading name"
              autocomplete="off"
              class="h-6 min-w-0 flex-1 px-1.5 text-2xs"
              disabled={!props.captureRootPath() || !props.selectedCaptureTarget() || props.isLoadingCapturePreview()}
              name="new-heading-name"
              onInput={(event) => setNewHeadingName(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                event.preventDefault();
                void submitNewHeading();
              }}
              placeholder="Heading name"
              value={newHeadingName()}
            />

            <Button
              class="h-6 px-1.5 text-2xs"
              disabled={!props.captureRootPath() || !props.selectedCaptureTarget() || props.isLoadingCapturePreview() || !newHeadingName().trim()}
              onClick={() => void submitNewHeading()}
              size="sm"
              type="button"
              variant="secondary"
            >
              Add
            </Button>
          </div>
        </div>

        {/* Heading list */}
        <div class="mt-1.5 min-h-0 flex-1 overflow-auto">
          <Show
            when={props.captureTargetH1ToH4().length > 0}
            fallback={
              <div class="flex flex-col items-center justify-center py-3 text-center">
                <svg
                  class="mb-1 h-5 w-5"
                  style={{ color: "var(--text-ghost)" }}
                  fill="none"
                  stroke="currentColor"
                  viewBox="0 0 24 24"
                >
                  <path
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    stroke-width="2"
                    d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z"
                  />
                </svg>
                <p class="text-2xs" style={{ color: "var(--text-ghost)" }}>No headings yet</p>
              </div>
            }
          >
            <DragDropProvider
              onDragEnd={(event) => {
                if (event.canceled) return;
                const sourceOrder = parseHeadingOrderFromDnd(
                  event.operation.source?.id ?? null,
                );
                const targetOrder = parseHeadingOrderFromDnd(
                  event.operation.target?.id ?? null,
                );
                if (
                  sourceOrder === null ||
                  targetOrder === null ||
                  sourceOrder === targetOrder
                )
                  return;
                void props.moveCaptureHeading(sourceOrder, targetOrder);
              }}
            >
              <div aria-label="Capture headings" class="space-y-0.5" role="list">
                <For each={props.captureTargetH1ToH4()}>
                  {(heading) => (
                    <PreviewHeadingRow
                      canMoveDown={Boolean(moveHintForHeading(heading.order)?.down)}
                      canMoveUp={Boolean(moveHintForHeading(heading.order)?.up)}
                      deleteCaptureHeading={props.deleteCaptureHeading}
                      heading={heading}
                      isBusy={props.isLoadingCapturePreview()}
                      isSelected={
                        props.selectedCaptureHeadingOrder() === heading.order
                      }
                      moveHeadingDown={moveHeadingDown}
                      moveHeadingUp={moveHeadingUp}
                      setSelectedCaptureHeadingOrder={
                        props.setSelectedCaptureHeadingOrder
                      }
                    />
                  )}
                </For>
              </div>
            </DragDropProvider>
          </Show>
        </div>

        <p class="mt-1.5 text-2xs" style={{ color: "var(--text-ghost)" }}>
          Click to set context &middot; Drag to reorder
        </p>
      </div>
    </div>
  );
}
