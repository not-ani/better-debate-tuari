import type { TexSessionSnapshot } from "./types";

export type OpenTab = {
  id: string;
  document: TexSessionSnapshot;
  dirty: boolean;
  saving: boolean;
  stickyHighlightMode: boolean;
  activeBlockIndex: number | null;
};

export const upsertSessionSnapshot = (tabs: OpenTab[], snapshot: TexSessionSnapshot) => {
  const existingIndex = tabs.findIndex((tab) => tab.id === snapshot.sessionId);
  if (existingIndex >= 0) {
    const next = tabs.slice();
    next[existingIndex] = {
      ...next[existingIndex]!,
      document: snapshot,
      dirty: snapshot.dirty,
      saving: false,
    };
    return next;
  }

  return [
    ...tabs,
    {
      id: snapshot.sessionId,
      document: snapshot,
      dirty: snapshot.dirty,
      saving: false,
      stickyHighlightMode: false,
      activeBlockIndex: null,
    },
  ];
};

export const updateTabDocument = (
  tabs: OpenTab[],
  tabId: string,
  document: TexSessionSnapshot,
) =>
  tabs.map((tab) =>
    tab.id === tabId
      ? {
          ...tab,
          document,
          dirty: true,
        }
      : tab,
  );

export const setTabStickyHighlightMode = (tabs: OpenTab[], tabId: string, enabled: boolean) =>
  tabs.map((tab) => (tab.id === tabId ? { ...tab, stickyHighlightMode: enabled } : tab));

export const setTabSaving = (tabs: OpenTab[], tabId: string, saving: boolean) =>
  tabs.map((tab) => (tab.id === tabId ? { ...tab, saving } : tab));

export const setTabActiveBlockIndex = (
  tabs: OpenTab[],
  tabId: string,
  activeBlockIndex: number | null,
) => tabs.map((tab) => (tab.id === tabId ? { ...tab, activeBlockIndex } : tab));

export const applySavedDocument = (
  tabs: OpenTab[],
  tabId: string,
  document: TexSessionSnapshot,
) =>
  tabs.map((tab) =>
    tab.id === tabId
      ? {
          ...tab,
          document,
          dirty: document.dirty,
          saving: false,
        }
      : tab,
  );

export const removeTab = (tabs: OpenTab[], tabId: string) => tabs.filter((tab) => tab.id !== tabId);

export const getCycledTabId = (
  tabs: OpenTab[],
  activeTabId: string | null,
  direction: 1 | -1,
) => {
  if (tabs.length === 0) {
    return null;
  }

  const activeIndex = tabs.findIndex((tab) => tab.id === activeTabId);
  const startIndex = activeIndex >= 0 ? activeIndex : 0;
  const nextIndex = (startIndex + direction + tabs.length) % tabs.length;
  return tabs[nextIndex]?.id ?? null;
};
