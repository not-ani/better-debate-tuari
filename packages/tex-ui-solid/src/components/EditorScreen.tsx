import { createSignal, For, Show } from "solid-js";
import DocumentEditor from "./DocumentEditor";
import { OutlineTree } from "./OutlineTree";
import type { OutlineNode } from "../lib/outline";
import type { OpenTab } from "../lib/workspace";
import type { TexSessionSnapshot } from "../lib/types";

type EditorScreenProps = {
  tab: OpenTab;
  tabs: OpenTab[];
  activeTabId: string | null;
  busy: boolean;
  canPopOut: boolean;
  outline: OutlineNode[];
  sidebarCollapsed: boolean;
  collapsedNodes: Set<number>;
  scrollTarget: number | null;
  invisibilityMode: boolean;
  stickyHighlightMode: boolean;
  speechSendOpen: boolean;
  onActiveBlockIndexChange: (blockIndex: number | null) => void;
  onToggleInvisibilityMode: () => void;
  onToggleStickyHighlightMode: () => void;
  onNewFile: () => void;
  onNewSpeech: () => void;
  onShowFiles: () => void;
  onOpenSearch: () => void;
  onOpenDialog: () => void;
  onOpenSpeechSend: (forceTargetPick: boolean) => void;
  onPopOut: () => void;
  onSave: () => void;
  onDocumentChange: (document: TexSessionSnapshot) => void;
  onToggleSidebar: () => void;
  onToggleOutlineNode: (blockIndex: number) => void;
  onOutlineClick: (blockIndex: number) => void;
  onSwitchTab: (tabId: string) => void;
  onCloseTab: (tabId: string) => void;
};

export default function EditorScreen(props: EditorScreenProps) {
  const [ribbonOpen, setRibbonOpen] = createSignal(true);

  return (
    <main class="editor-screen">
      {/* Row 1: Tab bar — Files / Open / file tabs */}
      <header class="tab-bar">
        <div class="tab-bar-left">
          <button class="tab-bar-menu" onClick={props.onShowFiles} type="button">
            Files
          </button>
          <button
            class="tab-bar-menu"
            disabled={props.busy}
            onClick={props.onOpenDialog}
            type="button"
          >
            Open
          </button>
          <button
            class="tab-bar-menu"
            disabled={props.busy}
            onClick={props.onNewSpeech}
            type="button"
          >
            New Speech
          </button>
          <button class="tab-bar-menu" onClick={props.onOpenSearch} type="button">
            Search
          </button>
          <Show when={props.canPopOut}>
            <button
              class="tab-bar-menu"
              disabled={props.busy}
              onClick={props.onPopOut}
              type="button"
            >
              Pop Out
            </button>
          </Show>
        </div>

        <div class="tab-bar-tabs">
          <For each={props.tabs}>
            {(tab) => (
              <button
                class="file-tab"
                classList={{ active: tab.id === props.activeTabId }}
                onClick={() => props.onSwitchTab(tab.id)}
                type="button"
              >
                <span
                  class="file-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    props.onCloseTab(tab.id);
                  }}
                >
                  &times;
                </span>
                <span class="file-tab-name">
                  {tab.document.fileName}
                  {tab.dirty ? " *" : ""}
                </span>
              </button>
            )}
          </For>
        </div>

        <div class="tab-bar-drag" />
      </header>

      {/* Main content: sidebar + (ribbon + editor) */}
      <div class="editor-layout">
        <aside class="outline-sidebar" classList={{ collapsed: props.sidebarCollapsed }}>
          <div class="outline-sidebar-header">
            <button
              class="sidebar-toggle"
              onClick={props.onToggleSidebar}
              title={props.sidebarCollapsed ? "Show outline" : "Hide outline"}
              type="button"
            >
              <svg
                width="10"
                height="10"
                viewBox="0 0 10 10"
                fill="none"
                class="sidebar-chevron-icon"
                classList={{ collapsed: props.sidebarCollapsed }}
              >
                <path
                  d="M6.5 2L3.5 5L6.5 8"
                  stroke="currentColor"
                  stroke-width="1.5"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                />
              </svg>
            </button>
            <Show when={!props.sidebarCollapsed}>
              <span class="outline-heading">Outline</span>
            </Show>
          </div>

          <Show when={!props.sidebarCollapsed}>
            <div class="outline-tree">
              <Show
                when={props.outline.length > 0}
                fallback={<p class="outline-empty">No headings in document.</p>}
              >
                <OutlineTree
                  nodes={props.outline}
                  collapsedNodes={props.collapsedNodes}
                  onToggle={props.onToggleOutlineNode}
                  onClick={props.onOutlineClick}
                />
              </Show>
            </div>
          </Show>
        </aside>

        <section class="editor-main">
          <DocumentEditor
            document={props.tab.document}
            invisibilityMode={props.invisibilityMode}
            stickyHighlightMode={props.stickyHighlightMode}
            speechSendOpen={props.speechSendOpen}
            onDocumentChange={props.onDocumentChange}
            onActiveBlockIndexChange={props.onActiveBlockIndexChange}
            onNewFile={props.onNewFile}
            onOpenSpeechSend={props.onOpenSpeechSend}
            onSave={props.onSave}
            onToggleInvisibilityMode={props.onToggleInvisibilityMode}
            onToggleStickyHighlightMode={props.onToggleStickyHighlightMode}
            saving={props.tab.saving}
            scrollToBlockIndex={props.scrollTarget}
            ribbonOpen={ribbonOpen()}
            onToggleRibbon={() => setRibbonOpen((v) => !v)}
          />
        </section>
      </div>
    </main>
  );
}
