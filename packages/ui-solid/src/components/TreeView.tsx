import { For, Show, type Accessor } from "solid-js";
import { Badge } from "./ui/badge";
import type { SearchHit, TreeRow } from "../lib/types";

type VirtualWindow = {
  topSpacerPx: number;
  bottomSpacerPx: number;
};

type TreeViewProps = {
  visibleTreeRows: Accessor<TreeRow[]>;
  virtualWindow: Accessor<VirtualWindow>;
  focusedNodeKey: Accessor<string>;
  searchMode: Accessor<boolean>;
  searchQuery: Accessor<string>;
  expandedFolders: Accessor<Set<string>>;
  expandedFiles: Accessor<Set<number>>;
  collapsedHeadings: Accessor<Set<string>>;
  activateRow: (row: TreeRow, fromKeyboard: boolean) => Promise<void>;
  applyPreviewFromRow: (row: TreeRow) => void;
  openSearchResult: (result: SearchHit) => Promise<void>;
  setFocusedNodeKey: (key: string) => void;
  onTreeKeyDown: (event: KeyboardEvent) => void;
  onTreeScroll: (scrollTop: number) => void;
  setTreeRef: (element: HTMLDivElement) => void;
  isLoadingSnapshot: Accessor<boolean>;
  treeRowsLength: Accessor<number>;
  selectedRootPath: Accessor<string>;
  isSearching: Accessor<boolean>;
};

const FileIcon = () => (
  <svg class="h-3.5 w-3.5" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M9 12h6m-6 4h6m2 5H7a2 2 0 01-2-2V5a2 2 0 012-2h5.586a1 1 0 01.707.293l5.414 5.414a1 1 0 01.293.707V19a2 2 0 01-2 2z" />
  </svg>
);

const ChevronRightIcon = () => (
  <svg class="h-2.5 w-2.5" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M9 5l7 7-7 7" />
  </svg>
);

const ChevronDownIcon = () => (
  <svg class="h-2.5 w-2.5" style={{ color: "var(--text-tertiary)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M19 9l-7 7-7-7" />
  </svg>
);

const HeadingIcon = () => (
  <svg class="h-3 w-3" style={{ color: "var(--accent)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 6h16M4 12h16M4 18h7" />
  </svg>
);

const F8Icon = () => (
  <span
    class="flex h-3.5 w-3.5 items-center justify-center rounded text-[8px] font-bold"
    style={{ background: "var(--amber-dim)", color: "var(--amber)" }}
  >
    F8
  </span>
);

const AuthorIcon = () => (
  <svg class="h-3 w-3" style={{ color: "var(--violet)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M16 7a4 4 0 11-8 0 4 4 0 018 0zM12 14a7 7 0 00-7 7h14a7 7 0 00-7-7z" />
  </svg>
);

export default function TreeView(props: TreeViewProps) {
  const trimmedSearchQuery = () => props.searchQuery().trim();
  const rowIsExpanded = (row: TreeRow) => {
    if (row.kind === "folder") return props.expandedFolders().has(row.folderPath ?? "");
    if (row.kind === "file" && !props.searchMode()) return props.expandedFiles().has(row.fileId ?? -1);
    if (row.kind === "heading" && row.hasChildren) return !props.collapsedHeadings().has(row.key);
    return undefined;
  };

  return (
    <div
      aria-busy={props.isLoadingSnapshot() || props.isSearching()}
      aria-label={props.searchMode() ? "Search results" : "Indexed document tree"}
      class="h-full min-h-0 overflow-auto px-1.5 pb-2 pt-1 focus-visible:outline-none"
      id="workspace-tree-scroll"
      role="tree"
      style={{ "scroll-padding": "4px" }}
      onKeyDown={props.onTreeKeyDown}
      onScroll={(event) => props.onTreeScroll(event.currentTarget.scrollTop)}
      ref={props.setTreeRef}
      tabindex={0}
    >
      <div aria-hidden="true" role="presentation" style={{ height: `${props.virtualWindow().topSpacerPx}px` }} />

      <For each={props.visibleTreeRows()}>
        {(row) => {
          const focused = () => props.focusedNodeKey() === row.key;
          const isFolder = () => row.kind === "folder";
          const isFile = () => row.kind === "file";
          const isHeading = () => row.kind === "heading";
          const isF8 = () => row.kind === "f8";
          const isAuthor = () => row.kind === "author";

          return (
            <button
              aria-expanded={rowIsExpanded(row)}
              aria-level={row.depth + 1}
              aria-selected={focused()}
              class={`tree-row group ${
                focused() ? "tree-row-focused" : ""
              } ${row.kind === "loading" ? "cursor-default opacity-40" : ""}`}
              data-row-key={row.key}
              onClick={() => {
                if (row.kind === "loading") return;
                if (props.searchMode() && row.searchResult) {
                  void props.openSearchResult(row.searchResult);
                  return;
                }
                void props.activateRow(row, false);
              }}
              onFocus={() => {
                if (row.kind !== "loading") {
                  props.setFocusedNodeKey(row.key);
                  props.applyPreviewFromRow(row);
                }
              }}
              onMouseEnter={() => {
                if (row.kind !== "loading") {
                  props.setFocusedNodeKey(row.key);
                }
                props.applyPreviewFromRow(row);
              }}
              role="treeitem"
              style={{ "padding-left": `${12 + row.depth * 16}px` }}
              type="button"
            >
              <span aria-hidden="true" class="flex h-4 w-4 shrink-0 items-center justify-center">
                {isFolder() ? (
                  props.expandedFolders().has(row.folderPath ?? "") ? (
                    <ChevronDownIcon />
                  ) : (
                    <ChevronRightIcon />
                  )
                ) : isFile() ? (
                  props.searchMode() ? (
                    <FileIcon />
                  ) : props.expandedFiles().has(row.fileId ?? -1) ? (
                    <ChevronDownIcon />
                  ) : (
                    <ChevronRightIcon />
                  )
                ) : isHeading() ? (
                  row.hasChildren ? (
                    props.collapsedHeadings().has(row.key) ? (
                      <ChevronRightIcon />
                    ) : (
                      <ChevronDownIcon />
                    )
                  ) : (
                    <HeadingIcon />
                  )
                ) : isF8() ? (
                  <F8Icon />
                ) : isAuthor() ? (
                  <AuthorIcon />
                ) : (
                  <span class="h-1 w-1 rounded-full" style={{ background: "var(--text-ghost)" }} />
                )}
              </span>
              
              <span class="min-w-0 flex-1">
                <p
                  class={`truncate text-2xs ${
                    isHeading() || isF8() || isAuthor()
                      ? "font-medium"
                      : ""
                  }`}
                  style={{
                    color: isHeading() || isF8() || isAuthor()
                      ? "var(--text-primary)"
                      : "var(--text-secondary)",
                  }}
                >
                  {row.label}
                </p>
                <Show when={row.subLabel}>
                  <p class="truncate text-2xs" style={{ color: "var(--text-ghost)" }}>{row.subLabel}</p>
                </Show>
              </span>

              <Show when={focused() && (isHeading() || isF8() || isAuthor())}>
                <Badge class="opacity-0 transition-opacity group-hover:opacity-100 motion-reduce:transition-none" variant="muted">
                  Space to insert
                </Badge>
              </Show>
              <Show when={props.searchMode() && row.searchResult && row.searchResult.source !== "lexical"}>
                <Badge variant="info">
                  {row.searchResult?.source === "mixed" ? "AI + Lex" : "AI Match"}
                </Badge>
              </Show>
            </button>
          );
        }}
      </For>

      <div aria-hidden="true" role="presentation" style={{ height: `${props.virtualWindow().bottomSpacerPx}px` }} />

      <Show when={!props.isLoadingSnapshot() && props.treeRowsLength() === 0 && !props.selectedRootPath()}>
        <div class="flex flex-col items-center justify-center py-10 text-center">
          <div
            aria-hidden="true"
            class="mb-3 flex h-10 w-10 items-center justify-center rounded-lg"
            style={{ background: "var(--surface-3)" }}
          >
            <svg class="h-5 w-5" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M5 19a2 2 0 01-2-2V7a2 2 0 012-2h4l2 2h4a2 2 0 012 2v1M5 19h14a2 2 0 002-2v-5a2 2 0 00-2-2H9a2 2 0 00-2 2v5a2 2 0 01-2 2z" />
            </svg>
          </div>
          <p class="text-xs font-medium" style={{ color: "var(--text-secondary)" }}>No folders indexed</p>
          <p class="mt-0.5 text-2xs" style={{ color: "var(--text-ghost)" }}>Add a folder to start building your document index.</p>
        </div>
      </Show>
      
      <Show when={!props.isLoadingSnapshot() && props.treeRowsLength() === 0 && props.searchMode()}>
        <div class="flex flex-col items-center justify-center py-10 text-center">
          <Show
            when={props.isSearching()}
            fallback={
              <>
                <div
                  aria-hidden="true"
                  class="mb-3 flex h-10 w-10 items-center justify-center rounded-lg"
                  style={{ background: "var(--surface-3)" }}
                >
                  <svg class="h-5 w-5" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                  </svg>
                </div>
                <p class="text-xs font-medium" style={{ color: "var(--text-secondary)" }}>No results for "{trimmedSearchQuery()}"</p>
                <p class="mt-0.5 text-2xs" style={{ color: "var(--text-ghost)" }}>Try a broader keyword, filename search, or AI semantic search.</p>
              </>
            }
          >
            <div
              aria-hidden="true"
              class="mb-3 flex h-10 w-10 items-center justify-center rounded-lg"
              style={{ background: "var(--surface-3)" }}
            >
              <div
                class="h-5 w-5 animate-spin rounded-full border-2"
                style={{ "border-color": "var(--surface-4)", "border-top-color": "var(--accent)" }}
              />
            </div>
            <p class="text-xs font-medium" style={{ color: "var(--text-secondary)" }}>Searching for "{trimmedSearchQuery()}"</p>
            <p class="mt-0.5 text-2xs" style={{ color: "var(--text-ghost)" }}>Results will appear here as soon as the index responds.</p>
          </Show>
        </div>
      </Show>
    </div>
  );
}
