import { Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { ProsemirrorAdapterProvider } from "@prosemirror-adapter/solid";
import { Menu, PredefinedMenuItem, Submenu } from "@tauri-apps/api/menu";
import { WebviewWindow, getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import CommandPalette, { type CommandPaletteItem } from "./components/CommandPalette";
import EditorScreen from "./components/EditorScreen";
import PickerScreen from "./components/PickerScreen";
import SpeechSendModal from "./components/SpeechSendModal";
import {
  RECENT_FILES_STORAGE_KEY,
  attachTexSession,
  createTexSpeechSessionDialog,
  createTexSessionDialog,
  getActiveSpeechTarget,
  discardTexRecoverableSession,
  listRecentFiles,
  listTexDetachedWindows,
  listTexOpenSessions,
  listTexRecoverableSessions,
  listenTexEvent,
  openTexSessionDialog,
  openTexSessionFromFile,
  prepareTexPopout,
  releaseTexSession,
  rememberRecentSession,
  saveTexSession,
  sendToTexSession,
  setActiveSpeechTarget,
  updateTexSession,
} from "./electrobun/bridge";
import { getInitialInvisibilityMode, persistInvisibilityMode } from "./lib/invisibility";
import { buildOutlineTree } from "./lib/outline";
import { buildSpeechSendSource, placementForTarget } from "./lib/speechRouting";
import { applyTheme, getInitialTheme, type ThemeMode } from "./lib/theme";
import type {
  RecentFile,
  TexRecoverableSession,
  TexSendInsertMode,
  TexSessionRouteTarget,
  TexSessionAttachResult,
  TexSessionOpenResult,
  TexSessionSnapshot,
  TexSessionUpdatedEvent,
} from "./lib/types";
import {
  buildDetachedWindowLabel,
  createDetachedWindow,
  getLaunchContext,
  type DetachedWindowEntry,
} from "./lib/windowing";
import {
  applySavedDocument,
  getCycledTabId,
  type OpenTab,
  removeTab,
  setTabActiveBlockIndex,
  setTabSaving,
  setTabStickyHighlightMode,
  updateTabDocument,
  upsertSessionSnapshot,
} from "./lib/workspace";

const SESSION_POPOUT_ATTACHED_EVENT = "tex://session-popout-attached";
const DETACHED_WINDOWS_CHANGED_EVENT = "tex://detached-windows-changed";
const RECOVERABLE_SESSIONS_CHANGED_EVENT = "tex://recoverable-sessions-changed";
const SESSION_UPDATED_EVENT = "tex://session-updated";
const SPEECH_TARGET_CHANGED_EVENT = "tex://speech-target-changed";
const SESSION_UPDATE_DEBOUNCE_MS = 250;
const POPOUT_ATTACH_TIMEOUT_MS = 5000;

const normalizeSearch = (value: string) => value.trim().toLowerCase();

const matchesQuery = (query: string, ...values: Array<string | undefined>) => {
  const normalized = normalizeSearch(query);
  if (!normalized) {
    return true;
  }

  const haystack = values.filter(Boolean).join(" ").toLowerCase();
  return normalized
    .split(/\s+/)
    .every((token) => haystack.includes(token));
};

const buildWindowCloseMessage = (dirtyTabs: OpenTab[]) => {
  const visibleNames = dirtyTabs.slice(0, 3).map((tab) => tab.document.fileName);
  const moreCount = Math.max(0, dirtyTabs.length - visibleNames.length);
  const suffix = moreCount > 0 ? ` and ${moreCount} more` : "";
  return `Close this window and discard unsaved changes in ${visibleNames.join(", ")}${suffix}?`;
};

const isOpenedResult = (
  result: TexSessionOpenResult | TexSessionAttachResult,
): result is
  | { kind: "opened"; snapshot: TexSessionSnapshot }
  | { kind: "attached"; snapshot: TexSessionSnapshot } =>
  result.kind === "opened" || result.kind === "attached";

export default function App() {
  const [recentFiles, setRecentFiles] = createSignal<RecentFile[]>([]);
  const [recoverableSessions, setRecoverableSessions] = createSignal<TexRecoverableSession[]>([]);
  const [tabs, setTabs] = createSignal<OpenTab[]>([]);
  const [activeTabId, setActiveTabId] = createSignal<string | null>(null);
  const [screen, setScreen] = createSignal<"picker" | "editor">("picker");
  const [sidebarCollapsed, setSidebarCollapsed] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [collapsedNodes, setCollapsedNodes] = createSignal<Set<number>>(new Set());
  const [scrollTarget, setScrollTarget] = createSignal<number | null>(null);
  const [invisibilityMode, setInvisibilityMode] = createSignal(getInitialInvisibilityMode());
  const [theme, setTheme] = createSignal<ThemeMode>(getInitialTheme());
  const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);
  const [commandPaletteQuery, setCommandPaletteQuery] = createSignal("");
  const [detachedWindows, setDetachedWindows] = createSignal<DetachedWindowEntry[]>([]);
  const [currentWindowLabel, setCurrentWindowLabel] = createSignal("main");
  const [isDetachedWindow, setIsDetachedWindow] = createSignal(false);
  const [speechSendOpen, setSpeechSendOpen] = createSignal(false);
  const [speechSendBusy, setSpeechSendBusy] = createSignal(false);
  const [speechRouteTargets, setSpeechRouteTargets] = createSignal<TexSessionRouteTarget[]>([]);
  const [selectedSpeechTargetSessionId, setSelectedSpeechTargetSessionId] = createSignal<string | null>(null);
  const [selectedSpeechTargetBlockIndex, setSelectedSpeechTargetBlockIndex] = createSignal<number | null>(null);
  const [speechRootSelected, setSpeechRootSelected] = createSignal(false);

  const launchContext = getLaunchContext();
  const pendingFlushTimers = new Map<string, number>();
  const flushInFlight = new Set<string>();
  const queuedFlushes = new Set<string>();
  const pendingPopoutAcks = new Map<
    string,
    {
      resolve: () => void;
      reject: (error: Error) => void;
      timeoutId: number;
    }
  >();
  let allowWindowClose = false;

  const activeTab = createMemo(() => tabs().find((tab) => tab.id === activeTabId()) ?? null);

  const outline = createMemo(() => {
    const tab = activeTab();
    if (!tab) {
      return [];
    }
    return buildOutlineTree(tab.document.blocks);
  });
  const speechSource = createMemo(() => {
    const tab = activeTab();
    if (!tab) {
      return { error: "No active document." } as const;
    }
    return buildSpeechSendSource(tab.document.blocks, tab.activeBlockIndex);
  });
  const selectedSpeechTarget = createMemo(
    () =>
      speechRouteTargets().find((target) => target.sessionId === selectedSpeechTargetSessionId()) ??
      null,
  );
  const selectedSpeechHeading = createMemo(() => {
    const blockIndex = selectedSpeechTargetBlockIndex();
    if (blockIndex == null) {
      return null;
    }
    return selectedSpeechTarget()?.headings.find((heading) => heading.blockIndex === blockIndex) ?? null;
  });
  const speechPlacementBelow = createMemo(() => {
    const source = speechSource();
    if ("error" in source) {
      return { allowed: false, reason: source.error };
    }
    return placementForTarget(
      speechRootSelected() ? null : selectedSpeechHeading(),
      "below",
      source.sourceMaxRelativeDepth,
    );
  });
  const speechPlacementUnder = createMemo(() => {
    const source = speechSource();
    if ("error" in source) {
      return { allowed: false, reason: source.error };
    }
    return placementForTarget(
      speechRootSelected() ? null : selectedSpeechHeading(),
      "under",
      source.sourceMaxRelativeDepth,
    );
  });

  const syncDetachedWindows = async () => {
    setDetachedWindows(await listTexDetachedWindows());
  };

  const syncRecoverableSessions = async () => {
    if (isDetachedWindow()) {
      setRecoverableSessions([]);
      return;
    }
    setRecoverableSessions(await listTexRecoverableSessions());
  };

  const loadRecent = async () => {
    setRecentFiles(await listRecentFiles());
  };

  const setupNativeMenu = async () => {
    const aboutSubmenu = await Submenu.new({
      text: "Tex",
      items: [
        await PredefinedMenuItem.new({
          item: {
            About: {
              name: "Tex",
            },
          },
        }),
        await PredefinedMenuItem.new({ item: "Separator" }),
        await PredefinedMenuItem.new({ item: "Quit" }),
      ],
    });

    const fileSubmenu = await Submenu.new({
      text: "File",
      items: [
        {
          id: "new-file",
          text: "New...",
          accelerator: "CmdOrCtrl+N",
          action: () => void handleNewFileDialog(),
        },
        {
          id: "open-file",
          text: "Open...",
          accelerator: "CmdOrCtrl+O",
          action: () => void handleOpenDialog(),
        },
        {
          id: "new-speech",
          text: "New Speech...",
          action: () => void handleNewSpeechDialog(),
        },
        {
          id: "save-file",
          text: "Save",
          accelerator: "CmdOrCtrl+S",
          action: () => void handleSaveActive(),
        },
        {
          id: "pop-out-file",
          text: "Pop Out Window",
          accelerator: "CmdOrCtrl+Shift+N",
          action: () => void handlePopOutActive(),
        },
      ],
    });

    const editSubmenu = await Submenu.new({
      text: "Edit",
      items: [
        await PredefinedMenuItem.new({ item: "Undo" }),
        await PredefinedMenuItem.new({ item: "Redo" }),
        await PredefinedMenuItem.new({ item: "Separator" }),
        await PredefinedMenuItem.new({ item: "Cut" }),
        await PredefinedMenuItem.new({ item: "Copy" }),
        await PredefinedMenuItem.new({ item: "Paste" }),
        await PredefinedMenuItem.new({ item: "SelectAll" }),
      ],
    });

    const windowSubmenu = await Submenu.new({
      text: "Window",
      items: [
        await PredefinedMenuItem.new({ item: "Minimize" }),
        await PredefinedMenuItem.new({ item: "Maximize" }),
        await PredefinedMenuItem.new({ item: "Separator" }),
        await PredefinedMenuItem.new({ item: "CloseWindow" }),
      ],
    });

    const menu = await Menu.new({
      items: [aboutSubmenu, fileSubmenu, editSubmenu, windowSubmenu],
    });

    await menu.setAsAppMenu();

    if (navigator.userAgent.toLowerCase().includes("mac")) {
      await windowSubmenu.setAsWindowsMenuForNSApp().catch(() => undefined);
    }
  };

  const updateWindowPresentation = async (tab: OpenTab | null) => {
    const currentWindow = getCurrentWebviewWindow();
    const title = tab ? `${tab.document.fileName} - Tex` : "Tex";
    await currentWindow.setTitle(title);
  };

  const markRecentSession = (snapshot: TexSessionSnapshot) => {
    setRecentFiles(rememberRecentSession(snapshot));
  };

  const acceptSessionSnapshot = (
    snapshot: TexSessionSnapshot,
    options?: { stickyHighlightMode?: boolean },
  ) => {
    markRecentSession(snapshot);
    setTabs((current) => {
      let next = upsertSessionSnapshot(current, snapshot);
      if (options?.stickyHighlightMode) {
        next = setTabStickyHighlightMode(next, snapshot.sessionId, true);
      }
      return next;
    });
    setActiveTabId(snapshot.sessionId);
    setScreen("editor");
  };

  const focusWindowByLabel = async (label: string) => {
    const targetWindow = await WebviewWindow.getByLabel(label);
    if (!targetWindow) {
      void syncDetachedWindows();
      return;
    }

    await targetWindow.show();
    await targetWindow.setFocus();
  };

  const handleOwnerConflict = async (result: {
    fileName: string;
    ownerWindowLabel: string;
  }) => {
    await focusWindowByLabel(result.ownerWindowLabel);
    window.alert(
      `${result.fileName} is already open in window ${result.ownerWindowLabel}. Tex keeps a single writer for each document session.`,
    );
  };

  const handleSessionResult = async (
    result: TexSessionOpenResult | TexSessionAttachResult,
    options?: { stickyHighlightMode?: boolean },
  ) => {
    if (isOpenedResult(result)) {
      acceptSessionSnapshot(result.snapshot, options);
      return;
    }

    await handleOwnerConflict(result);
  };

  const removeFlushTimer = (sessionId: string) => {
    const timeoutId = pendingFlushTimers.get(sessionId);
    if (timeoutId !== undefined) {
      window.clearTimeout(timeoutId);
      pendingFlushTimers.delete(sessionId);
    }
  };

  const flushSession = async (sessionId: string) => {
    removeFlushTimer(sessionId);

    if (flushInFlight.has(sessionId)) {
      queuedFlushes.add(sessionId);
      return;
    }

    const tab = tabs().find((candidate) => candidate.id === sessionId);
    if (!tab || !tab.dirty) {
      return;
    }

    flushInFlight.add(sessionId);
    const documentToSend = tab.document;

    try {
      const snapshot = await updateTexSession({
        sessionId,
        windowLabel: currentWindowLabel(),
        baseVersion: documentToSend.version,
        document: documentToSend,
      });

      setTabs((current) => {
        const latest = current.find((candidate) => candidate.id === sessionId);
        if (!latest) {
          return current;
        }

        if (latest.document === documentToSend) {
          return updateTabDocument(current, sessionId, snapshot);
        }

        return updateTabDocument(current, sessionId, {
          ...latest.document,
          version: snapshot.version,
          dirty: true,
        });
      });
    } catch (error) {
      console.error("Could not flush Tex session update", error);
    } finally {
      flushInFlight.delete(sessionId);
      if (queuedFlushes.delete(sessionId)) {
        void flushSession(sessionId);
      }
    }
  };

  const flushSessionNow = async (sessionId: string | null) => {
    if (!sessionId) {
      return;
    }

    removeFlushTimer(sessionId);
    await flushSession(sessionId);

    for (let attempt = 0; attempt < 200; attempt += 1) {
      if (!flushInFlight.has(sessionId) && !pendingFlushTimers.has(sessionId)) {
        return;
      }
      await new Promise((resolve) => window.setTimeout(resolve, 25));
    }
  };

  const scheduleSessionFlush = (sessionId: string) => {
    removeFlushTimer(sessionId);
    pendingFlushTimers.set(
      sessionId,
      window.setTimeout(() => {
        pendingFlushTimers.delete(sessionId);
        void flushSession(sessionId);
      }, SESSION_UPDATE_DEBOUNCE_MS),
    );
  };

  const closePalette = () => {
    setCommandPaletteOpen(false);
    setCommandPaletteQuery("");
  };

  const openPalette = () => {
    setCommandPaletteQuery("");
    setCommandPaletteOpen(true);
  };

  const finalizeLocalTabClose = (tabId: string) => {
    removeFlushTimer(tabId);
    queuedFlushes.delete(tabId);

    const remaining = removeTab(tabs(), tabId);
    setTabs(remaining);

    if (remaining.length === 0) {
      setActiveTabId(null);
      setScreen("picker");
      void loadRecent();
      return;
    }

    if (activeTabId() === tabId) {
      const fallbackTab = remaining[Math.max(remaining.length - 1, 0)] ?? null;
      setActiveTabId(fallbackTab?.id ?? null);
    }
  };

  const releaseAndCloseTab = async (
    tabId: string,
    options?: {
      discardUnsaved?: boolean;
      confirmDiscard?: boolean;
    },
  ) => {
    const target = tabs().find((tab) => tab.id === tabId);
    if (!target) {
      return;
    }

    const discardUnsaved = options?.discardUnsaved ?? target.dirty;
    const confirmDiscard = options?.confirmDiscard ?? discardUnsaved;
    if (
      confirmDiscard &&
      discardUnsaved &&
      !window.confirm(`Close ${target.document.fileName} without saving?`)
    ) {
      return;
    }

    if (!discardUnsaved) {
      await flushSessionNow(tabId);
    }

    try {
      await releaseTexSession(tabId, currentWindowLabel(), discardUnsaved);
    } catch (error) {
      console.error("Could not release Tex session", error);
    }

    finalizeLocalTabClose(tabId);
  };

  const cycleTabs = (direction: 1 | -1) => {
    const nextTabId = getCycledTabId(tabs(), activeTabId(), direction);
    if (!nextTabId) {
      return;
    }

    const previousTabId = activeTabId();
    if (previousTabId && previousTabId !== nextTabId) {
      void flushSessionNow(previousTabId);
    }
    setActiveTabId(nextTabId);
    setScreen("editor");
  };

  const handleOpenDialog = async () => {
    if (busy()) {
      return;
    }

    setBusy(true);
    try {
      const result = await openTexSessionDialog(currentWindowLabel());
      if (result) {
        await handleSessionResult(result);
      }
    } finally {
      setBusy(false);
    }
  };

  const handleNewFileDialog = async () => {
    if (busy()) {
      return;
    }

    setBusy(true);
    try {
      const result = await createTexSessionDialog(currentWindowLabel());
      if (result) {
        await handleSessionResult(result);
      }
    } catch (error) {
      console.error("Could not create a new Tex document", error);
      window.alert(
        `Could not create a new document.\n\n${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setBusy(false);
    }
  };

  const loadSpeechTargets = async (options?: { forcePickTarget?: boolean }) => {
    const [targets, targetState] = await Promise.all([
      listTexOpenSessions(),
      getActiveSpeechTarget(),
    ]);
    setSpeechRouteTargets(targets);

    const rememberedTarget =
      !options?.forcePickTarget && targetState.targetSessionId
        ? targets.find((target) => target.sessionId === targetState.targetSessionId)?.sessionId ?? null
        : null;

    setSelectedSpeechTargetSessionId(rememberedTarget);
    setSelectedSpeechTargetBlockIndex(null);
    setSpeechRootSelected(false);
  };

  const openSpeechSend = async (forcePickTarget = false) => {
    if (busy() || speechSendBusy()) {
      return;
    }

    const source = speechSource();
    if ("error" in source) {
      window.alert(source.error);
      return;
    }

    setSpeechSendBusy(true);
    try {
      await loadSpeechTargets({ forcePickTarget });
      setSpeechSendOpen(true);
    } catch (error) {
      console.error("Could not load speech targets", error);
      window.alert(
        `Could not load speech targets.\n\n${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setSpeechSendBusy(false);
    }
  };

  const closeSpeechSend = () => {
    if (speechSendBusy()) {
      return;
    }
    setSpeechSendOpen(false);
    setSelectedSpeechTargetBlockIndex(null);
    setSpeechRootSelected(false);
  };

  const handleSpeechTargetSessionSelect = (sessionId: string) => {
    setSelectedSpeechTargetSessionId(sessionId);
    setSelectedSpeechTargetBlockIndex(null);
    setSpeechRootSelected(false);
  };

  const handleSpeechRootSelect = () => {
    setSpeechRootSelected(true);
    setSelectedSpeechTargetBlockIndex(null);
  };

  const handleSpeechTargetBlockSelect = (blockIndex: number) => {
    setSpeechRootSelected(false);
    setSelectedSpeechTargetBlockIndex(blockIndex);
  };

  const handleSendToSpeech = async (insertMode: TexSendInsertMode) => {
    const source = speechSource();
    if ("error" in source) {
      window.alert(source.error);
      return;
    }

    const targetSessionId = selectedSpeechTargetSessionId();
    if (!targetSessionId) {
      window.alert("Choose a target document first.");
      return;
    }

    const placement = insertMode === "below" ? speechPlacementBelow() : speechPlacementUnder();
    if (!placement.allowed) {
      window.alert(placement.reason ?? "This send is unavailable.");
      return;
    }

    const localTargetTab = tabs().find((tab) => tab.id === targetSessionId);
    if (localTargetTab?.dirty) {
      await flushSessionNow(localTargetTab.id);
    }

    setSpeechSendBusy(true);
    try {
      const result = await sendToTexSession({
        targetSessionId,
        targetBlockIndex: speechRootSelected() ? null : selectedSpeechTargetBlockIndex(),
        insertMode,
        sourceBlocks: source.sourceBlocks,
        sourceRootLevel: source.sourceRootLevel,
        sourceMaxRelativeDepth: source.sourceMaxRelativeDepth,
      });
      await setActiveSpeechTarget(targetSessionId);
      setTabs((current) => {
        const existing = current.find((tab) => tab.id === result.snapshot.sessionId);
        if (!existing) {
          return current;
        }
        return upsertSessionSnapshot(current, result.snapshot);
      });
      closeSpeechSend();
    } catch (error) {
      console.error("Could not send to speech target", error);
      window.alert(
        `Could not send to speech target.\n\n${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setSpeechSendBusy(false);
    }
  };

  const handleNewSpeechDialog = async () => {
    if (busy()) {
      return;
    }

    setBusy(true);
    try {
      const result = await createTexSpeechSessionDialog(currentWindowLabel());
      if (result) {
        await handleSessionResult(result);
      }
    } catch (error) {
      console.error("Could not create a new speech document", error);
      window.alert(
        `Could not create a new speech document.\n\n${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setBusy(false);
    }
  };

  const handleOpenRecent = async (path: string) => {
    if (busy()) {
      return;
    }

    setBusy(true);
    try {
      await handleSessionResult(await openTexSessionFromFile(path, currentWindowLabel()));
    } finally {
      setBusy(false);
    }
  };

  const handleRecoverSession = async (sessionId: string) => {
    if (busy()) {
      return;
    }

    setBusy(true);
    try {
      await handleSessionResult(await attachTexSession(sessionId, currentWindowLabel(), "writer"));
      await syncRecoverableSessions();
    } finally {
      setBusy(false);
    }
  };

  const handleDiscardRecovery = async (sessionId: string) => {
    setBusy(true);
    try {
      await discardTexRecoverableSession(sessionId);
      await syncRecoverableSessions();
    } finally {
      setBusy(false);
    }
  };

  const handlePopOutActive = async () => {
    const target = activeTab();
    if (!target || busy() || isDetachedWindow()) {
      return;
    }

    setBusy(true);
    await flushSessionNow(target.id);

    const detachedLabel = buildDetachedWindowLabel(target.document.filePath);
    const popoutKey = `${target.id}:${detachedLabel}`;
    const ackPromise = new Promise<void>((resolve, reject) => {
      const timeoutId = window.setTimeout(() => {
        pendingPopoutAcks.delete(popoutKey);
        reject(new Error("Detached window did not attach in time."));
      }, POPOUT_ATTACH_TIMEOUT_MS);

      pendingPopoutAcks.set(popoutKey, {
        resolve: () => {
          window.clearTimeout(timeoutId);
          pendingPopoutAcks.delete(popoutKey);
          resolve();
        },
        reject: (error) => {
          window.clearTimeout(timeoutId);
          pendingPopoutAcks.delete(popoutKey);
          reject(error);
        },
        timeoutId,
      });
    });

    try {
      const prepared = await prepareTexPopout(target.id, currentWindowLabel(), detachedLabel);
      await createDetachedWindow(prepared);
      await ackPromise;
      await releaseAndCloseTab(target.id, {
        discardUnsaved: false,
        confirmDiscard: false,
      });
    } catch (error) {
      const pendingAck = pendingPopoutAcks.get(popoutKey);
      if (pendingAck) {
        window.clearTimeout(pendingAck.timeoutId);
        pendingPopoutAcks.delete(popoutKey);
      }
      console.error("Could not create detached window", error);
      window.alert(
        `Could not open a detached window.\n\n${error instanceof Error ? error.message : String(error)}`,
      );
    } finally {
      setBusy(false);
    }
  };

  const updateActiveDocument = (nextDocument: TexSessionSnapshot) => {
    const activeId = activeTabId();
    if (!activeId) {
      return;
    }

    setTabs((current) => updateTabDocument(current, activeId, nextDocument));
    scheduleSessionFlush(activeId);
  };

  const updateActiveBlockIndex = (blockIndex: number | null) => {
    const activeId = activeTabId();
    if (!activeId) {
      return;
    }
    setTabs((current) => setTabActiveBlockIndex(current, activeId, blockIndex));
  };

  const handleSaveTab = async (tabId: string) => {
    const target = tabs().find((tab) => tab.id === tabId);
    if (!target || target.saving) {
      return;
    }

    setTabs((current) => setTabSaving(current, tabId, true));

    try {
      await flushSessionNow(tabId);
      const snapshot = await saveTexSession(tabId, currentWindowLabel());
      markRecentSession(snapshot);
      setTabs((current) => applySavedDocument(current, tabId, snapshot));
    } finally {
      setTabs((current) => setTabSaving(current, tabId, false));
    }
  };

  const handleSaveActive = async () => {
    const target = activeTab();
    if (!target) {
      return;
    }

    await handleSaveTab(target.id);
  };

  const toggleTheme = () => {
    setTheme((current) => (current === "light" ? "dark" : "light"));
  };

  const toggleInvisibilityMode = () => {
    setInvisibilityMode((current) => !current);
  };

  const toggleStickyHighlightMode = () => {
    const target = activeTab();
    if (!target) {
      return;
    }

    setTabs((current) =>
      setTabStickyHighlightMode(current, target.id, !target.stickyHighlightMode),
    );
  };

  const handleOutlineClick = (blockIndex: number) => {
    setScrollTarget(null);
    queueMicrotask(() => setScrollTarget(blockIndex));
  };

  const toggleCollapse = (blockIndex: number) => {
    setCollapsedNodes((prev) => {
      const next = new Set(prev);
      if (next.has(blockIndex)) {
        next.delete(blockIndex);
      } else {
        next.add(blockIndex);
      }
      return next;
    });
  };

  const handleSwitchTab = (tabId: string) => {
    const previousTabId = activeTabId();
    if (previousTabId && previousTabId !== tabId) {
      void flushSessionNow(previousTabId);
    }
    setActiveTabId(tabId);
    setScreen("editor");
  };

  const commandPaletteItems = createMemo<CommandPaletteItem[]>(() => {
    const query = commandPaletteQuery();
    const detachedByPath = new Set(detachedWindows().map((entry) => entry.filePath));
    const items: CommandPaletteItem[] = [];

    items.push({
      id: "command:new-file",
      badge: "Command",
      title: "New file...",
      subtitle: "Create a new document and choose where to save it",
      run: () => void handleNewFileDialog(),
    });

    items.push({
      id: "command:new-speech",
      badge: "Command",
      title: "New speech...",
      subtitle: "Create a new speech document and choose where to save it",
      run: () => void handleNewSpeechDialog(),
    });

    items.push({
      id: "command:open-file",
      badge: "Command",
      title: "Open file...",
      subtitle: "Browse for a document on disk",
      run: () => void handleOpenDialog(),
    });

    if (activeTab()) {
      items.push({
        id: "command:send-speech",
        badge: "Command",
        title: "Send to speech...",
        subtitle: "Choose a target Tex doc and destination heading",
        run: () => void openSpeechSend(false),
      });
    }

    const currentActiveTab = activeTab();
    if (currentActiveTab && !isDetachedWindow()) {
      items.push({
        id: "command:pop-out",
        badge: "Command",
        title: `Pop out ${currentActiveTab.document.fileName}`,
        subtitle: "Move the active file into its own window",
        run: () => void handlePopOutActive(),
      });
    }

    for (const tab of tabs()) {
      if (!matchesQuery(query, tab.document.fileName, tab.document.filePath, "open tab")) {
        continue;
      }

      items.push({
        id: `tab:${tab.id}`,
        badge: "Open",
        title: tab.document.fileName,
        subtitle: tab.document.filePath,
        meta: tab.id === activeTabId() ? "Active" : undefined,
        run: () => handleSwitchTab(tab.id),
      });
    }

    for (const entry of detachedWindows()) {
      if (entry.label === currentWindowLabel()) {
        continue;
      }

      if (!matchesQuery(query, entry.fileName, entry.filePath, "window detached")) {
        continue;
      }

      items.push({
        id: `window:${entry.label}`,
        badge: "Window",
        title: entry.fileName,
        subtitle: entry.filePath,
        meta: "Detached",
        run: () => void focusWindowByLabel(entry.label),
      });
    }

    for (const file of recentFiles()) {
      if (!matchesQuery(query, file.name, file.path, "recent file")) {
        continue;
      }

      items.push({
        id: `recent:${file.path}`,
        badge: "Recent",
        title: file.name,
        subtitle: file.path,
        meta: detachedByPath.has(file.path) ? "Open elsewhere" : undefined,
        run: async () => {
          await handleOpenRecent(file.path);
        },
      });
    }

    return items.filter((item) =>
      matchesQuery(query, item.title, item.subtitle, item.badge, item.meta),
    );
  });

  onMount(() => {
    const currentWindow = getCurrentWebviewWindow();
    setCurrentWindowLabel(currentWindow.label);
    setIsDetachedWindow(launchContext.mode === "detached");
    void loadRecent();
    void syncDetachedWindows();
    void syncRecoverableSessions();
    void setupNativeMenu();

    const onStorage = (event: StorageEvent) => {
      if (!event.key || event.key === RECENT_FILES_STORAGE_KEY) {
        void loadRecent();
      }
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === "p") {
        event.preventDefault();
        openPalette();
        return;
      }

      if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === "n") {
        event.preventDefault();
        void handleNewFileDialog();
        return;
      }

      if (event.ctrlKey && event.key === "Tab") {
        event.preventDefault();
        cycleTabs(event.shiftKey ? -1 : 1);
        return;
      }

      if (
        activeTab() &&
        !event.altKey &&
        event.shiftKey &&
        (event.metaKey || event.ctrlKey) &&
        event.key.toLowerCase() === "i"
      ) {
        event.preventDefault();
        toggleInvisibilityMode();
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "o") {
        event.preventDefault();
        void handleOpenDialog();
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "s" && activeTab()) {
        event.preventDefault();
        void handleSaveActive();
        return;
      }

      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "w" && activeTab()) {
        event.preventDefault();
        void releaseAndCloseTab(activeTab()!.id);
      }
    };

    const onBlur = () => {
      void flushSessionNow(activeTabId());
    };

    window.addEventListener("storage", onStorage);
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("blur", onBlur);

    const closeUnlistenPromise = currentWindow.onCloseRequested((event) => {
      if (allowWindowClose) {
        return;
      }

      event.preventDefault();
      const openTabs = tabs();
      const dirtyTabs = openTabs.filter((tab) => tab.dirty);
      if (dirtyTabs.length > 0 && !window.confirm(buildWindowCloseMessage(dirtyTabs))) {
        return;
      }

      allowWindowClose = true;
      void (async () => {
        for (const tab of openTabs) {
          removeFlushTimer(tab.id);
          try {
            await releaseTexSession(tab.id, currentWindow.label, tab.dirty);
          } catch (error) {
            console.error("Could not release Tex session during window close", error);
          }
        }
        await currentWindow.close();
      })();
    });

    const detachedWindowsUnlistenPromise = listenTexEvent<DetachedWindowEntry[]>(
      DETACHED_WINDOWS_CHANGED_EVENT,
      (entries) => {
        setDetachedWindows(entries);
      },
    );
    const recoverableUnlistenPromise = listenTexEvent<TexRecoverableSession[]>(
      RECOVERABLE_SESSIONS_CHANGED_EVENT,
      (entries) => {
        if (!isDetachedWindow()) {
          setRecoverableSessions(entries);
        }
      },
    );
    const popoutUnlistenPromise = listenTexEvent<{
      sessionId: string;
      fromWindowLabel: string;
      toWindowLabel: string;
    }>(SESSION_POPOUT_ATTACHED_EVENT, (payload) => {
      const pendingAck = pendingPopoutAcks.get(`${payload.sessionId}:${payload.toWindowLabel}`);
      pendingAck?.resolve();
    });
    const sessionUpdatedUnlistenPromise = listenTexEvent<TexSessionUpdatedEvent>(
      SESSION_UPDATED_EVENT,
      ({ snapshot }) => {
        setTabs((current) => {
          const existing = current.find((tab) => tab.id === snapshot.sessionId);
          if (!existing) {
            return current;
          }
          return upsertSessionSnapshot(current, snapshot);
        });
        if (speechSendOpen()) {
          void listTexOpenSessions().then((targets) => setSpeechRouteTargets(targets));
        }
      },
    );
    const speechTargetChangedUnlistenPromise = listenTexEvent<{ targetSessionId: string | null }>(
      SPEECH_TARGET_CHANGED_EVENT,
      (payload) => {
        if (
          payload.targetSessionId &&
          speechSendOpen() &&
          !selectedSpeechTargetSessionId()
        ) {
          setSelectedSpeechTargetSessionId(payload.targetSessionId);
        }
      },
    );

    if (launchContext.mode === "detached" && launchContext.sessionId) {
      setBusy(true);
      void attachTexSession(launchContext.sessionId, currentWindow.label, "writer")
        .then((result) => handleSessionResult(result, { stickyHighlightMode: true }))
        .catch((error) => {
          console.error("Could not attach detached Tex session", error);
          window.alert(
            `Could not open the detached session.\n\n${error instanceof Error ? error.message : String(error)}`,
          );
        })
        .finally(() => setBusy(false));
    }

    onCleanup(() => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("blur", onBlur);

      for (const timeoutId of pendingFlushTimers.values()) {
        window.clearTimeout(timeoutId);
      }
      pendingFlushTimers.clear();

      for (const pendingAck of pendingPopoutAcks.values()) {
        window.clearTimeout(pendingAck.timeoutId);
        pendingAck.reject(new Error("The window was destroyed before pop-out completed."));
      }
      pendingPopoutAcks.clear();

      void closeUnlistenPromise.then((unlisten) => unlisten());
      void detachedWindowsUnlistenPromise.then((unlisten) => unlisten());
      void recoverableUnlistenPromise.then((unlisten) => unlisten());
      void popoutUnlistenPromise.then((unlisten) => unlisten());
      void sessionUpdatedUnlistenPromise.then((unlisten) => unlisten());
      void speechTargetChangedUnlistenPromise.then((unlisten) => unlisten());
    });
  });

  createEffect(() => {
    applyTheme(theme());
  });

  createEffect(() => {
    persistInvisibilityMode(invisibilityMode());
  });

  createEffect(() => {
    activeTabId();
    setCollapsedNodes(new Set<number>());
  });

  createEffect(() => {
    void updateWindowPresentation(activeTab());
  });

  return (
    <ProsemirrorAdapterProvider>
      <Show
        when={screen() === "editor" && activeTab()}
        fallback={
          <PickerScreen
            busy={busy()}
            recentFiles={recentFiles()}
            recoverableSessions={recoverableSessions()}
            theme={theme()}
            onDiscardRecovery={(sessionId) => void handleDiscardRecovery(sessionId)}
            onNewSpeech={() => void handleNewSpeechDialog()}
            onOpenDialog={() => void handleOpenDialog()}
            onOpenRecent={(path) => void handleOpenRecent(path)}
            onOpenSearch={openPalette}
            onRecoverSession={(sessionId) => void handleRecoverSession(sessionId)}
            onToggleTheme={toggleTheme}
          />
        }
      >
        {(tab) => (
          <EditorScreen
            activeTabId={activeTabId()}
            busy={busy()}
            canPopOut={Boolean(activeTab()) && !isDetachedWindow()}
            collapsedNodes={collapsedNodes()}
            invisibilityMode={invisibilityMode()}
            onCloseTab={(tabId) => void releaseAndCloseTab(tabId)}
            onDocumentChange={updateActiveDocument}
            onActiveBlockIndexChange={updateActiveBlockIndex}
            onNewFile={() => void handleNewFileDialog()}
            onNewSpeech={() => void handleNewSpeechDialog()}
            onOpenDialog={() => void handleOpenDialog()}
            onOpenSpeechSend={(forceTargetPick) => void openSpeechSend(forceTargetPick)}
            onOpenSearch={openPalette}
            onOutlineClick={handleOutlineClick}
            onPopOut={() => void handlePopOutActive()}
            onSave={() => void handleSaveActive()}
            onShowFiles={() => setScreen("picker")}
            onSwitchTab={handleSwitchTab}
            onToggleInvisibilityMode={toggleInvisibilityMode}
            onToggleOutlineNode={toggleCollapse}
            onToggleSidebar={() => setSidebarCollapsed((value) => !value)}
            onToggleStickyHighlightMode={toggleStickyHighlightMode}
            outline={outline()}
            scrollTarget={scrollTarget()}
            sidebarCollapsed={sidebarCollapsed()}
            speechSendOpen={speechSendOpen()}
            stickyHighlightMode={tab().stickyHighlightMode}
            tab={tab()}
            tabs={tabs()}
          />
        )}
      </Show>

      <CommandPalette
        items={commandPaletteItems()}
        onClose={closePalette}
        onExecute={async (item) => {
          closePalette();
          await item.run();
        }}
        onQueryChange={setCommandPaletteQuery}
        open={commandPaletteOpen()}
        query={commandPaletteQuery()}
      />

      <SpeechSendModal
        belowReason={speechPlacementBelow().reason}
        busy={speechSendBusy()}
        onClose={closeSpeechSend}
        onConfirm={(insertMode) => void handleSendToSpeech(insertMode)}
        onSelectRoot={handleSpeechRootSelect}
        onSelectTargetBlock={handleSpeechTargetBlockSelect}
        onSelectTargetSession={handleSpeechTargetSessionSelect}
        open={speechSendOpen()}
        rootSelected={speechRootSelected()}
        selectedTargetBlockIndex={selectedSpeechTargetBlockIndex()}
        selectedTargetSessionId={selectedSpeechTargetSessionId()}
        sourceLabel={
          "error" in speechSource()
            ? "Move the caret onto a Pocket, Hat, Block, or Tag heading to send."
            : speechSource().sourceHeadingText
        }
        targets={speechRouteTargets()}
        underReason={speechPlacementUnder().reason}
      />
    </ProsemirrorAdapterProvider>
  );
}
