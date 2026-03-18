import type { Accessor, Setter } from "solid-js";
import CaptureTargetPanel from "./CaptureTargetPanel";
import SidePreviewPane from "./SidePreviewPane";
import TopControls from "./TopControls";
import TreeView from "./TreeView";
import type { DesktopUpdateStatus } from "../electrobun/bridge";
import type {
  CaptureTarget,
  CaptureTargetPreview,
  FileHeading,
  IndexProgress,
  RootSummary,
  SearchHit,
  SidePreview,
  TreeRow,
} from "../lib/types";

type VirtualWindow = {
  start: number;
  end: number;
  topSpacerPx: number;
  bottomSpacerPx: number;
};

type AppWorkspaceProps = {
  activeLastIndexedMs: Accessor<number>;
  activeRootLabel: Accessor<string>;
  openIndexesPage: () => void;
  copyToast: Accessor<string>;
  isIndexing: Accessor<boolean>;
  isSearching: Accessor<boolean>;
  roots: Accessor<RootSummary[]>;
  runIndexForSelection: () => Promise<void>;
  searchQuery: Accessor<string>;
  searchFileNamesOnly: Accessor<boolean>;
  searchDebatifyEnabled: Accessor<boolean>;
  searchSemanticEnabled: Accessor<boolean>;
  selectedRootPath: Accessor<string>;
  indexProgress: Accessor<IndexProgress | null>;
  setSearchInputRef: (element: HTMLInputElement) => void;
  setSearchQuery: (value: string) => void;
  setSelectedRootPath: (value: string) => void;
  toggleFileNameSearchMode: () => void;
  toggleDebatifySearchMode: () => void;
  toggleSemanticSearchMode: () => void;
  updateStatus: Accessor<DesktopUpdateStatus | null>;
  isCheckingUpdates: Accessor<boolean>;
  isInstallingUpdate: Accessor<boolean>;
  checkForUpdates: () => Promise<void>;
  installUpdateNow: () => Promise<void>;
  openPath: (path: string) => Promise<void>;
  status: Accessor<string>;
  showCapturePanel: Accessor<boolean>;
  showPreviewPanel: Accessor<boolean>;
  toggleCapturePanel: () => void;
  togglePreviewPanel: () => void;
  leftRailWidthPx: Accessor<number>;
  startLeftRailResize: (event: MouseEvent) => void;
  addCaptureHeading: (headingLevel: 1 | 2 | 3 | 4, headingName: string) => Promise<boolean>;
  captureRootPath: Accessor<string>;
  captureTargetH1ToH4: Accessor<FileHeading[]>;
  captureTargetPreview: Accessor<CaptureTargetPreview | null>;
  captureTargets: Accessor<CaptureTarget[]>;
  createCaptureTarget: () => Promise<void>;
  deleteCaptureHeading: (headingOrder: number) => Promise<void>;
  isAllRootsSelected: Accessor<boolean>;
  isLoadingCapturePreview: Accessor<boolean>;
  isLoadingCaptureTargets: Accessor<boolean>;
  moveCaptureHeading: (sourceHeadingOrder: number, targetHeadingOrder: number) => Promise<void>;
  selectCaptureTargetFromFilesystem: () => Promise<void>;
  selectedCaptureHeadingOrder: Accessor<number | null>;
  selectedCaptureTarget: Accessor<string>;
  selectedCaptureTargetMeta: Accessor<CaptureTarget | null>;
  setSelectedCaptureHeadingOrder: Setter<number | null>;
  setSelectedCaptureTarget: (value: string, persist?: boolean) => void;
  activateRow: (row: TreeRow, fromKeyboard?: boolean) => Promise<void>;
  applyPreviewFromRow: (row: TreeRow) => void;
  collapsedHeadings: Accessor<Set<string>>;
  expandedFiles: Accessor<Set<number>>;
  expandedFolders: Accessor<Set<string>>;
  focusedNodeKey: Accessor<string>;
  isLoadingSnapshot: Accessor<boolean>;
  onTreeKeyDown: (event: KeyboardEvent) => void;
  onTreeScroll: (scrollTop: number) => void;
  openSearchResult: (result: SearchHit) => Promise<void>;
  searchMode: Accessor<boolean>;
  setFocusedNodeKey: (key: string) => void;
  setTreeRef: (element: HTMLDivElement) => void;
  treeRowsLength: Accessor<number>;
  virtualWindow: Accessor<VirtualWindow>;
  visibleTreeRows: Accessor<TreeRow[]>;
  startPreviewPanelResize: (event: MouseEvent) => void;
  sidePreview: Accessor<SidePreview | null>;
  previewPanelWidthPx: Accessor<number>;
};

export default function AppWorkspace(props: AppWorkspaceProps) {
  return (
    <div class="app-shell h-screen overflow-hidden">
      <a class="skip-link" href="#workspace-tree">Skip to document tree</a>
      <div class="flex h-full w-full flex-col">
        <TopControls
          activeLastIndexedMs={props.activeLastIndexedMs}
          activeRootLabel={props.activeRootLabel}
          openIndexesPage={props.openIndexesPage}
          copyToast={props.copyToast}
          indexProgress={props.indexProgress}
          isIndexing={props.isIndexing}
          isSearching={props.isSearching}
          roots={props.roots}
          runIndexForSelection={props.runIndexForSelection}
          searchDebatifyEnabled={props.searchDebatifyEnabled}
          searchFileNamesOnly={props.searchFileNamesOnly}
          searchQuery={props.searchQuery}
          searchSemanticEnabled={props.searchSemanticEnabled}
          selectedRootPath={props.selectedRootPath}
          setSearchInputRef={props.setSearchInputRef}
          setSearchQuery={props.setSearchQuery}
          setSelectedRootPath={props.setSelectedRootPath}
          showCapturePanel={props.showCapturePanel}
          showPreviewPanel={props.showPreviewPanel}
          status={props.status}
          toggleCapturePanel={props.toggleCapturePanel}
          toggleDebatifySearchMode={props.toggleDebatifySearchMode}
          toggleFileNameSearchMode={props.toggleFileNameSearchMode}
          togglePreviewPanel={props.togglePreviewPanel}
          toggleSemanticSearchMode={props.toggleSemanticSearchMode}
          updateStatus={props.updateStatus}
          isCheckingUpdates={props.isCheckingUpdates}
          isInstallingUpdate={props.isInstallingUpdate}
          checkForUpdates={props.checkForUpdates}
          installUpdateNow={props.installUpdateNow}
          openPath={props.openPath}
        />

        <div class="flex min-h-0 flex-1">
          <div
            class="workspace-split h-full min-h-0 min-w-0 flex-1"
            style={{ "--left-rail-width": props.showCapturePanel() ? `${props.leftRailWidthPx()}px` : "0px" }}
          >
            {props.showCapturePanel() && (
              <aside
                aria-label="Capture target manager"
                class="panel-surface flex h-full min-h-0 flex-col border-r"
                style={{ borderRightColor: "var(--border-dim)" }}
              >
                <CaptureTargetPanel
                  addCaptureHeading={props.addCaptureHeading}
                  captureRootPath={props.captureRootPath}
                  captureTargetH1ToH4={props.captureTargetH1ToH4}
                  captureTargetPreview={props.captureTargetPreview}
                  captureTargets={props.captureTargets}
                  createCaptureTarget={props.createCaptureTarget}
                  deleteCaptureHeading={props.deleteCaptureHeading}
                  isAllRootsSelected={props.isAllRootsSelected}
                  isLoadingCapturePreview={props.isLoadingCapturePreview}
                  isLoadingCaptureTargets={props.isLoadingCaptureTargets}
                  moveCaptureHeading={props.moveCaptureHeading}
                  selectCaptureTargetFromFilesystem={props.selectCaptureTargetFromFilesystem}
                  selectedCaptureHeadingOrder={props.selectedCaptureHeadingOrder}
                  selectedCaptureTarget={props.selectedCaptureTarget}
                  selectedCaptureTargetMeta={props.selectedCaptureTargetMeta}
                  setSelectedCaptureHeadingOrder={props.setSelectedCaptureHeadingOrder}
                  setSelectedCaptureTarget={props.setSelectedCaptureTarget}
                />
              </aside>
            )}

            {props.showCapturePanel() && (
              <button
                aria-label="Resize insert preview panel"
                aria-orientation="vertical"
                class="panel-resize-handle hidden lg:flex"
                onMouseDown={props.startLeftRailResize}
                role="separator"
                title="Drag to resize"
                type="button"
              />
            )}

            <main class="relative h-full min-h-0 min-w-0 flex-1" id="workspace-tree">
              {props.isIndexing() && (
                <div
                  aria-live="polite"
                  class="pointer-events-none absolute right-3 top-3 z-10 flex max-w-[340px] items-center gap-2 rounded-full px-3 py-1.5 text-2xs shadow-lg"
                  style={{ background: "color-mix(in srgb, var(--surface-1) 84%, transparent)", border: "1px solid var(--accent-subtle)", color: "var(--accent-bright)" }}
                >
                  <div aria-hidden="true" class="h-2.5 w-2.5 animate-pulse rounded-full motion-reduce:animate-none" style={{ background: "var(--accent)" }} />
                  <span class="truncate">
                    {props.indexProgress()?.phase === "indexing"
                      ? `Indexing ${props.indexProgress()?.processed ?? 0}/${props.indexProgress()?.changed ?? 0}`
                      : props.indexProgress()?.phase === "discovering"
                        ? "Scanning files"
                        : props.indexProgress()?.phase === "committing"
                          ? "Committing database changes"
                        : props.indexProgress()?.phase === "lexical"
                          ? "Building lexical index"
                          : props.indexProgress()?.phase === "search"
                            ? "Updating search index"
                            : "Indexing in progress"}
                  </span>
                </div>
              )}
              <TreeView
                activateRow={props.activateRow}
                applyPreviewFromRow={props.applyPreviewFromRow}
                collapsedHeadings={props.collapsedHeadings}
                expandedFiles={props.expandedFiles}
                expandedFolders={props.expandedFolders}
                focusedNodeKey={props.focusedNodeKey}
                isLoadingSnapshot={props.isLoadingSnapshot}
                isSearching={props.isSearching}
                onTreeKeyDown={props.onTreeKeyDown}
                onTreeScroll={props.onTreeScroll}
                openSearchResult={props.openSearchResult}
                searchMode={props.searchMode}
                searchQuery={props.searchQuery}
                selectedRootPath={props.selectedRootPath}
                setFocusedNodeKey={props.setFocusedNodeKey}
                setTreeRef={props.setTreeRef}
                treeRowsLength={props.treeRowsLength}
                virtualWindow={props.virtualWindow}
                visibleTreeRows={props.visibleTreeRows}
              />
            </main>
          </div>

          {props.showPreviewPanel() && (
            <>
              <button
                aria-label="Resize preview panel"
                aria-orientation="vertical"
                class="panel-resize-handle flex"
                onMouseDown={props.startPreviewPanelResize}
                role="separator"
                title="Drag to resize preview"
                type="button"
              />
              <SidePreviewPane sidePreview={props.sidePreview} width={props.previewPanelWidthPx} />
            </>
          )}
        </div>
      </div>
    </div>
  );
}
