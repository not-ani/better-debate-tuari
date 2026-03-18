import { For, Show, createEffect, createMemo, createSignal, type Accessor } from "solid-js";
import type { IndexProgress, RootIndexEntry } from "../lib/types";
import {
  getIndexProgressForRoot,
  getProgressLabelForRoot,
  getProgressPercentForRoot,
} from "../lib/indexProgress";
import { formatBytes, formatTime } from "../lib/utils";
import IndexProgressPanel from "./IndexProgressPanel";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { Select } from "./ui/select";

type IndexesPageProps = {
  rootIndexes: Accessor<RootIndexEntry[]>;
  selectedRootPath: Accessor<string>;
  setSelectedRootPath: (value: string) => void;
  isIndexing: Accessor<boolean>;
  indexProgress: Accessor<IndexProgress | null>;
  status: Accessor<string>;
  addFolder: () => Promise<void>;
  reindexSelected: () => Promise<void>;
  refreshIndexes: () => Promise<void>;
  deleteIndex: (rootPath: string) => Promise<void>;
  openWorkspace: () => void;
  openPath: (path: string) => Promise<void>;
};

export default function IndexesPage(props: IndexesPageProps) {
  const [pendingDeleteRootPath, setPendingDeleteRootPath] = createSignal<string | null>(null);
  const selectedIndex = createMemo(() =>
    props.rootIndexes().find((entry) => entry.rootPath === props.selectedRootPath()) ?? null,
  );
  const activeIndexProgress = () => (props.isIndexing() ? props.indexProgress() : null);

  createEffect(() => {
    const pendingRootPath = pendingDeleteRootPath();
    if (!pendingRootPath) return;

    const stillExists = props.rootIndexes().some((entry) => entry.rootPath === pendingRootPath);
    if (!stillExists || props.isIndexing()) {
      setPendingDeleteRootPath(null);
    }
  });

  return (
    <div class="app-shell h-screen overflow-hidden">
      <a class="skip-link" href="#indexes-main">Skip to indexed folders</a>

      <div class="mx-auto flex h-full w-full max-w-[1440px] flex-col px-3 py-3 md:px-4 md:py-4">
        <header class="panel-surface-elevated rounded-2xl px-3 py-3">
          <div class="flex flex-wrap items-center gap-3">
            <Button onClick={props.openWorkspace} size="sm" type="button" variant="ghost">
              <svg aria-hidden="true" class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M15 19l-7-7 7-7" />
              </svg>
              Workspace
            </Button>

            <div class="min-w-0">
              <p class="section-label">Indexing</p>
              <h1 class="truncate text-sm font-semibold" style={{ color: "var(--text-primary)" }}>
                Indexed folders
              </h1>
              <p class="text-2xs" style={{ color: "var(--text-ghost)" }}>
                Manage source roots, reindex selectively, and inspect on-disk index artifacts.
              </p>
            </div>

            <div class="ml-auto flex flex-wrap items-end gap-2">
              <div class="flex min-w-[220px] flex-col">
                <label class="section-label" for="indexes-root-selector">
                  Selected Folder
                </label>
                <Select
                  aria-label="Select indexed folder"
                  class="mt-1 h-9 text-xs"
                  id="indexes-root-selector"
                  onChange={(event) => props.setSelectedRootPath(event.currentTarget.value)}
                  value={props.selectedRootPath()}
                >
                  <option value="">Select folder</option>
                  <For each={props.rootIndexes()}>
                    {(entry) => <option value={entry.rootPath}>{entry.folderName}</option>}
                  </For>
                </Select>
              </div>

              <div class="flex flex-wrap items-center gap-1.5">
                <Button
                  disabled={props.isIndexing()}
                  onClick={() => void props.addFolder()}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  <svg aria-hidden="true" class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
                  </svg>
                  Add Folder
                </Button>
                <Button
                  disabled={!selectedIndex() || props.isIndexing()}
                  onClick={() => void props.reindexSelected()}
                  size="sm"
                  type="button"
                  variant="default"
                >
                  <Show when={props.isIndexing()} fallback="Reindex Selected">
                    <>
                      <div aria-hidden="true" class="h-3 w-3 animate-spin rounded-full border-2 motion-reduce:animate-none" style={{ borderColor: "rgba(255,255,255,0.3)", borderTopColor: "white" }} />
                      Indexing
                    </>
                  </Show>
                </Button>
                <Button
                  disabled={props.isIndexing()}
                  onClick={() => void props.refreshIndexes()}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Refresh
                </Button>
              </div>
            </div>
          </div>

          <div
            aria-atomic="true"
            aria-live="polite"
            class="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-xl px-3 py-2"
            style={{ border: "1px solid var(--border-dim)", background: "var(--surface-0)" }}
          >
            <div class="flex items-center gap-2 text-2xs" style={{ color: "var(--text-secondary)" }}>
              <Badge variant={props.isIndexing() ? "info" : "muted"}>
                {props.isIndexing() ? "Indexing" : "Idle"}
              </Badge>
              <span class="truncate">{props.status()}</span>
            </div>
            <span class="metric-chip text-2xs" style={{ color: "var(--text-ghost)" }}>
              {props.rootIndexes().length.toLocaleString()} folders
            </span>
          </div>

          <Show when={activeIndexProgress()}>
            <div class="mt-3">
              <IndexProgressPanel
                class="rounded-xl px-3 py-3"
                includeDocxLabel
                openPath={props.openPath}
                progress={activeIndexProgress}
                showLogPath
              />
            </div>
          </Show>
        </header>

        <main class="mt-3 min-h-0 flex-1" id="indexes-main">
          <section class="panel-surface h-full overflow-hidden rounded-2xl">
            <Show
              when={props.rootIndexes().length > 0}
              fallback={
                <div class="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
                  <div
                    aria-hidden="true"
                    class="rounded-full p-3"
                    style={{ border: "1px solid var(--border-default)", background: "var(--surface-2)" }}
                  >
                    <svg class="h-6 w-6" style={{ color: "var(--text-ghost)" }} fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 7v10a2 2 0 002 2h14a2 2 0 002-2V9a2 2 0 00-2-2h-6l-2-2H5a2 2 0 00-2 2z" />
                    </svg>
                  </div>
                  <div>
                    <p class="text-sm font-medium" style={{ color: "var(--text-primary)" }}>No indexed folders yet</p>
                    <p class="mt-1 text-2xs" style={{ color: "var(--text-ghost)" }}>
                      Add a source root to start building local search and capture indexes.
                    </p>
                  </div>
                  <Button disabled={props.isIndexing()} onClick={() => void props.addFolder()} type="button" variant="default">
                    <svg aria-hidden="true" class="h-3 w-3" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                      <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M12 4v16m8-8H4" />
                    </svg>
                    Add First Folder
                  </Button>
                </div>
              }
            >
              <div class="h-full overflow-auto px-2 py-2">
                <div class="grid gap-2 md:grid-cols-2 xl:grid-cols-3">
                  <For each={props.rootIndexes()}>
                    {(entry) => {
                      const isSelected = () => props.selectedRootPath() === entry.rootPath;
                      const progress = () => getIndexProgressForRoot(activeIndexProgress(), entry.rootPath);
                      const progressPercent = () => getProgressPercentForRoot(activeIndexProgress(), entry.rootPath);

                      return (
                        <article
                          class="panel-surface-elevated rounded-2xl p-3"
                          style={{
                            background: isSelected()
                              ? "linear-gradient(180deg, rgba(45,212,191,0.08), transparent 30%), var(--surface-2)"
                              : undefined,
                            borderColor: isSelected() ? "var(--accent-subtle)" : undefined,
                          }}
                        >
                          <div class="flex items-start gap-3">
                            <div class="min-w-0 flex-1">
                              <div class="flex flex-wrap items-center gap-2">
                                <h2 class="truncate text-sm font-semibold" style={{ color: "var(--text-primary)" }}>
                                  {entry.folderName}
                                </h2>
                                <Show when={isSelected()}>
                                  <Badge variant="success">Selected</Badge>
                                </Show>
                              </div>
                              <p class="mt-1 truncate text-2xs" style={{ color: "var(--text-ghost)" }} title={entry.rootPath}>
                                {entry.rootPath}
                              </p>
                            </div>

                            <div class="flex flex-wrap justify-end gap-1">
                              <Button
                                onClick={() => props.setSelectedRootPath(entry.rootPath)}
                                size="sm"
                                type="button"
                                variant={isSelected() ? "default" : "ghost"}
                              >
                                {isSelected() ? "Active" : "Select"}
                              </Button>
                              <Button
                                onClick={() => void props.openPath(entry.indexPath)}
                                size="sm"
                                type="button"
                                variant="ghost"
                              >
                                Open Index
                              </Button>
                              <Show
                                when={pendingDeleteRootPath() === entry.rootPath}
                                fallback={
                                  <Button
                                    disabled={props.isIndexing()}
                                    onClick={() => setPendingDeleteRootPath(entry.rootPath)}
                                    size="sm"
                                    type="button"
                                    variant="ghost"
                                  >
                                    Delete
                                  </Button>
                                }
                              >
                                <Button
                                  class="text-[var(--rose)]"
                                  disabled={props.isIndexing()}
                                  onClick={() => {
                                    const deletion = props.deleteIndex(entry.rootPath);
                                    void deletion.finally(() => setPendingDeleteRootPath(null));
                                  }}
                                  size="sm"
                                  type="button"
                                  variant="ghost"
                                >
                                  Confirm
                                </Button>
                                <Button
                                  disabled={props.isIndexing()}
                                  onClick={() => setPendingDeleteRootPath(null)}
                                  size="sm"
                                  type="button"
                                  variant="ghost"
                                >
                                  Cancel
                                </Button>
                              </Show>
                            </div>
                          </div>

                          <div class="mt-3 flex flex-wrap gap-1.5 text-2xs" style={{ color: "var(--text-secondary)" }}>
                            <span class="metric-chip">{entry.fileCount.toLocaleString()} files</span>
                            <span class="metric-chip">{entry.headingCount.toLocaleString()} headings</span>
                            <span class="metric-chip">{formatBytes(entry.indexSizeBytes)}</span>
                            <span class="metric-chip">{formatTime(entry.lastIndexedMs)}</span>
                          </div>

                          <Show when={progress()}>
                            <div class="mt-3 rounded-xl px-2.5 py-2" style={{ border: "1px solid var(--accent-subtle)", background: "var(--accent-dim)" }}>
                              <div class="flex items-center justify-between gap-2">
                                <span class="truncate text-2xs" style={{ color: "var(--accent-bright)" }}>
                                  {getProgressLabelForRoot(activeIndexProgress(), entry.rootPath)}
                                </span>
                                <Show when={progressPercent() !== null}>
                                  <span class="metric-chip text-2xs" style={{ color: "var(--accent)" }}>
                                    {progressPercent()}%
                                  </span>
                                </Show>
                              </div>
                              <div class="mt-2 h-1.5 overflow-hidden rounded-full" style={{ background: "var(--surface-4)" }}>
                                <Show
                                  when={progressPercent() !== null}
                                  fallback={<div class="h-full w-1/3 animate-pulse rounded-full motion-reduce:animate-none" style={{ background: "var(--accent-fg)" }} />}
                                >
                                  <div
                                    class="h-full rounded-full transition-[width] duration-200 motion-reduce:transition-none"
                                    style={{ width: `${progressPercent() ?? 0}%`, background: "var(--accent)" }}
                                  />
                                </Show>
                              </div>
                            </div>
                          </Show>
                        </article>
                      );
                    }}
                  </For>
                </div>
              </div>
            </Show>
          </section>
        </main>
      </div>
    </div>
  );
}
