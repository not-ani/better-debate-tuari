import { For, Show, type Accessor } from "solid-js";
import type { DesktopUpdateStatus } from "../electrobun/bridge";
import { ALL_ROOTS_KEY } from "../lib/constants";
import type { IndexProgress, RootSummary } from "../lib/types";
import { formatTime } from "../lib/utils";
import IndexProgressPanel from "./IndexProgressPanel";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { Select } from "./ui/select";

type TopControlsProps = {
  searchQuery: Accessor<string>;
  searchFileNamesOnly: Accessor<boolean>;
  searchDebatifyEnabled: Accessor<boolean>;
  searchSemanticEnabled: Accessor<boolean>;
  setSearchQuery: (value: string) => void;
  setSearchInputRef: (element: HTMLInputElement) => void;
  isIndexing: Accessor<boolean>;
  openIndexesPage: () => void;
  selectedRootPath: Accessor<string>;
  runIndexForSelection: () => Promise<void>;
  activeRootLabel: Accessor<string>;
  activeLastIndexedMs: Accessor<number>;
  isSearching: Accessor<boolean>;
  status: Accessor<string>;
  copyToast: Accessor<string>;
  roots: Accessor<RootSummary[]>;
  setSelectedRootPath: (value: string) => void;
  indexProgress: Accessor<IndexProgress | null>;
  showCapturePanel: Accessor<boolean>;
  showPreviewPanel: Accessor<boolean>;
  toggleCapturePanel: () => void;
  togglePreviewPanel: () => void;
  toggleFileNameSearchMode: () => void;
  toggleDebatifySearchMode: () => void;
  toggleSemanticSearchMode: () => void;
  updateStatus: Accessor<DesktopUpdateStatus | null>;
  isCheckingUpdates: Accessor<boolean>;
  isInstallingUpdate: Accessor<boolean>;
  checkForUpdates: () => Promise<void>;
  installUpdateNow: () => Promise<void>;
  openPath: (path: string) => Promise<void>;
};

type TogglePillProps = {
  active: boolean;
  onClick: () => void;
  label: string;
  activeColor: string;
  title: string;
};

function TogglePill(props: TogglePillProps) {
  return (
    <button
      aria-label={props.title}
      aria-pressed={props.active}
      class={`h-6 rounded-full border px-2 text-2xs font-medium transition-colors motion-reduce:transition-none ${
        props.active
          ? `${props.activeColor} border-current/20 bg-current/10`
          : "border-transparent text-ghost hover:text-tertiary"
      }`}
      onClick={props.onClick}
      title={props.title}
      type="button"
    >
      {props.label}
    </button>
  );
}

export default function TopControls(props: TopControlsProps) {
  const activeIndexProgress = () => (props.isIndexing() ? props.indexProgress() : null);

  const showUpdateBanner = () => {
    const update = props.updateStatus();
    if (props.isCheckingUpdates() || props.isInstallingUpdate()) return true;
    if (!update) return false;
    if (update.error || update.updateReady || update.info?.updateAvailable) return true;
    return update.status === "checking-for-update";
  };

  const updateBannerText = () => {
    if (props.isInstallingUpdate()) return "Installing update…";
    if (props.isCheckingUpdates()) return "Checking for updates…";

    const update = props.updateStatus();
    if (!update) return "";
    if (update.error) return `Update failed: ${update.error}`;
    if (update.updateReady) return `v${update.info?.version ?? "?"} ready to install`;
    if (update.info?.updateAvailable) return `v${update.info.version || "?"} downloading…`;
    return update.message || "Checking…";
  };

  return (
    <header class="panel-surface border-b">
      <div class="flex flex-col gap-3 px-3 py-3">
        <div class="flex flex-wrap items-center gap-2">
          <div class="flex min-w-0 items-center gap-2 pr-2">
            <div
              aria-hidden="true"
              class="flex h-8 w-8 items-center justify-center rounded-xl border"
              style={{ background: "var(--accent-dim)", borderColor: "var(--accent-subtle)" }}
            >
              <svg aria-hidden="true" class="h-4 w-4" style={{ color: "var(--accent)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2.5" d="M19 11H5m14 0a2 2 0 012 2v6a2 2 0 01-2 2H5a2 2 0 01-2-2v-6a2 2 0 012-2m14 0V9a2 2 0 00-2-2M5 11V9a2 2 0 012-2m0 0V5a2 2 0 012-2h6a2 2 0 012 2v2M7 7h10" />
              </svg>
            </div>
            <div class="min-w-0">
              <p class="section-label">Workspace</p>
              <p class="truncate text-sm font-semibold" style={{ color: "var(--text-primary)" }}>
                BlockVault
              </p>
            </div>
          </div>

          <div class="min-w-[280px] flex-1">
            <label class="section-label" for="app-search-input">
              Search
            </label>
            <div class="relative mt-1">
              <div class="pointer-events-none absolute inset-y-0 left-0 flex items-center pl-3">
                <svg aria-hidden="true" class="h-3.5 w-3.5 text-ghost" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
                </svg>
              </div>
              <Input
                aria-label="Search indexed files, headings, and cards"
                autocomplete="off"
                class="h-9 pl-9 pr-10 text-xs"
                id="app-search-input"
                name="workspace-search"
                onInput={(event) => props.setSearchQuery(event.currentTarget.value)}
                placeholder={props.searchFileNamesOnly() ? "Filename search (F)" : "Search cards, headings, and citations (/)"}
                ref={props.setSearchInputRef}
                spellcheck={false}
                value={props.searchQuery()}
              />
              <Show when={props.isSearching()}>
                <div class="pointer-events-none absolute inset-y-0 right-0 flex items-center pr-3">
                  <div aria-hidden="true" class="h-3.5 w-3.5 animate-spin rounded-full border border-ghost motion-reduce:animate-none" style={{ borderTopColor: "var(--accent)" }} />
                </div>
              </Show>
            </div>
          </div>

          <div class="flex min-w-[220px] flex-col">
            <label class="section-label" for="root-selector">
              Scope
            </label>
            <Select
              aria-label="Select indexed folder scope"
              class="mt-1 h-9 text-xs"
              id="root-selector"
              onChange={(event) => props.setSelectedRootPath(event.currentTarget.value)}
              value={props.selectedRootPath()}
            >
              <option value={ALL_ROOTS_KEY}>All folders</option>
              <For each={props.roots()}>{(root) => <option value={root.path}>{root.path}</option>}</For>
            </Select>
          </div>
        </div>

        <div class="flex flex-wrap items-end gap-3">
          <div class="flex flex-col gap-1">
            <span class="section-label">Search Modes</span>
            <div class="flex flex-wrap items-center gap-1">
              <TogglePill
                active={props.searchFileNamesOnly()}
                activeColor="text-[var(--blue)]"
                label="File"
                onClick={props.toggleFileNameSearchMode}
                title="Toggle filename-only search"
              />
              <TogglePill
                active={props.searchDebatifyEnabled()}
                activeColor="text-[var(--accent)]"
                label="API"
                onClick={props.toggleDebatifySearchMode}
                title="Toggle Debatify API search"
              />
              <TogglePill
                active={props.searchSemanticEnabled()}
                activeColor="text-[var(--violet)]"
                label="AI"
                onClick={props.toggleSemanticSearchMode}
                title="Toggle semantic search"
              />
            </div>
          </div>

          <div class="flex flex-col gap-1">
            <span class="section-label">Panels</span>
            <div class="flex flex-wrap items-center gap-1">
              <TogglePill
                active={props.showCapturePanel()}
                activeColor="text-[var(--accent)]"
                label="Insert"
                onClick={props.toggleCapturePanel}
                title="Toggle insert panel"
              />
              <TogglePill
                active={props.showPreviewPanel()}
                activeColor="text-[var(--accent)]"
                label="Preview"
                onClick={props.togglePreviewPanel}
                title="Toggle preview panel"
              />
            </div>
          </div>

          <div class="ml-auto flex flex-wrap items-center gap-1.5">
            <Button
              onClick={props.openIndexesPage}
              size="sm"
              type="button"
              variant="ghost"
            >
              <svg aria-hidden="true" class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z" />
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 12a3 3 0 11-6 0 3 3 0 016 0z" />
              </svg>
              Indexes
            </Button>
            <Button
              disabled={!props.selectedRootPath() || props.isIndexing()}
              onClick={() => void props.runIndexForSelection()}
              size="sm"
              type="button"
              variant="outline"
            >
              <Show
                when={props.isIndexing()}
                fallback={
                  <>
                    <svg aria-hidden="true" class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                    </svg>
                    Reindex
                  </>
                }
              >
                <>
                  <div aria-hidden="true" class="h-3 w-3 animate-spin rounded-full border border-ghost motion-reduce:animate-none" style={{ borderTopColor: "var(--accent)" }} />
                  Indexing
                </>
              </Show>
            </Button>
          </div>
        </div>
      </div>

      <div
        aria-atomic="true"
        aria-live="polite"
        class="flex flex-wrap items-center gap-2 border-t px-3 py-2"
        style={{ borderColor: "var(--border-dim)", background: "var(--surface-0)" }}
      >
        <Show when={props.isSearching()}>
          <Badge variant="info">Searching</Badge>
        </Show>
        <Show when={props.copyToast()}>
          <Badge variant="success">{props.copyToast()}</Badge>
        </Show>
        <span class="flex-1 truncate text-2xs text-tertiary">{props.status()}</span>
        <span class="metric-chip text-2xs" style={{ color: "var(--text-ghost)" }}>
          {props.activeRootLabel()}
        </span>
        <span class="metric-chip text-2xs" style={{ color: "var(--text-ghost)" }}>
          Indexed {formatTime(props.activeLastIndexedMs())}
        </span>
      </div>

      <Show when={activeIndexProgress()}>
        <div class="border-t px-3 py-3" style={{ borderColor: "var(--border-dim)" }}>
          <IndexProgressPanel class="rounded-xl px-3 py-3" openPath={props.openPath} progress={activeIndexProgress} />
        </div>
      </Show>

      <Show when={showUpdateBanner()}>
        <div
          aria-atomic="true"
          aria-live="polite"
          class="flex flex-wrap items-center justify-between gap-2 border-t px-3 py-2"
          style={{ borderColor: "var(--border-dim)", background: "var(--blue-dim)" }}
        >
          <span class="truncate text-2xs text-primary">{updateBannerText()}</span>
          <div class="flex items-center gap-1 shrink-0">
            <Button
              disabled={props.isCheckingUpdates() || props.isInstallingUpdate()}
              onClick={() => void props.checkForUpdates()}
              size="sm"
              type="button"
              variant="ghost"
            >
              Check
            </Button>
            <Show when={props.updateStatus()?.updateReady}>
              <Button
                disabled={props.isInstallingUpdate()}
                onClick={() => void props.installUpdateNow()}
                size="sm"
                type="button"
                variant="default"
              >
                Install
              </Button>
            </Show>
          </div>
        </div>
      </Show>
    </header>
  );
}
