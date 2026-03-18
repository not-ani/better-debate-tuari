import { expect, test } from "bun:test";
import type { TexSessionSnapshot } from "./types";
import {
  getCycledTabId,
  setTabStickyHighlightMode,
  updateTabDocument,
  upsertSessionSnapshot,
} from "./workspace";

const createSession = (sessionId: string, filePath: string, fileName: string): TexSessionSnapshot => ({
  sessionId,
  version: 0,
  dirty: false,
  filePath,
  fileName,
  paragraphCount: 1,
  blocks: [],
});

test("upsertOpenResult initializes stickyHighlightMode to false", () => {
  const tabs = upsertSessionSnapshot([], createSession("session-1", "/tmp/one.tex", "one.tex"));

  expect(tabs).toHaveLength(1);
  expect(tabs[0]?.stickyHighlightMode).toBeFalse();
});

test("setTabStickyHighlightMode updates only the targeted tab", () => {
  const first = createSession("session-1", "/tmp/one.tex", "one.tex");
  const second = createSession("session-2", "/tmp/two.tex", "two.tex");
  const tabs = upsertSessionSnapshot(upsertSessionSnapshot([], first), second);

  const next = setTabStickyHighlightMode(tabs, "session-2", true);

  expect(next[0]?.stickyHighlightMode).toBeFalse();
  expect(next[1]?.stickyHighlightMode).toBeTrue();
});

test("updateTabDocument preserves stickyHighlightMode for the updated tab", () => {
  const session = createSession("session-1", "/tmp/one.tex", "one.tex");
  const tabs = setTabStickyHighlightMode(upsertSessionSnapshot([], session), "session-1", true);

  const nextDocument = createSession("session-1", "/tmp/one.tex", "one-renamed.tex");
  const next = updateTabDocument(tabs, "session-1", { ...nextDocument, dirty: true });

  expect(next[0]?.stickyHighlightMode).toBeTrue();
  expect(next[0]?.document.fileName).toBe("one-renamed.tex");
});

test("getCycledTabId wraps forward and backward across tabs", () => {
  const first = createSession("session-1", "/tmp/one.tex", "one.tex");
  const second = createSession("session-2", "/tmp/two.tex", "two.tex");
  const third = createSession("session-3", "/tmp/three.tex", "three.tex");
  const tabs = upsertSessionSnapshot(upsertSessionSnapshot(upsertSessionSnapshot([], first), second), third);

  expect(getCycledTabId(tabs, "session-3", 1)).toBe("session-1");
  expect(getCycledTabId(tabs, "session-1", -1)).toBe("session-3");
});
