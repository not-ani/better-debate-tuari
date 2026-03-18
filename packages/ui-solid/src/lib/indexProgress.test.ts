import { expect, test } from "bun:test";
import {
  getEstimatedTimeRemaining,
  getIndexProgressPercent,
  getIndexProgressStageLabel,
  getIndexProgressTitle,
} from "./indexProgress";
import type { IndexProgress } from "./types";

const progress = (overrides: Partial<IndexProgress> = {}): IndexProgress => ({
  rootPath: "/tmp/root",
  phase: "indexing",
  discovered: 12,
  changed: 10,
  processed: 4,
  updated: 0,
  skipped: 0,
  removed: 0,
  elapsedMs: 15_000,
  phaseElapsedMs: 8_000,
  scanRatePerSec: 12.3,
  processRatePerSec: 5.6,
  etaMs: 30_000,
  logPath: null,
  currentFile: "speech.docx",
  ...overrides,
});

test("caps in-flight indexing percent below 100", () => {
  expect(getIndexProgressPercent(progress())).toBe(40);
  expect(getIndexProgressPercent(progress({ processed: 10 }))).toBe(100);
});

test("formats progress titles by phase", () => {
  expect(getIndexProgressTitle(progress())).toBe("Indexing 4 / 10 files (6 left)");
  expect(getIndexProgressTitle(progress({ phase: "discovering" }), { includeDocxLabel: true })).toBe(
    "Scanning 12 .docx files",
  );
});

test("reports eta only while actively indexing", () => {
  expect(getEstimatedTimeRemaining(progress())).toBe("30s");
  expect(getEstimatedTimeRemaining(progress({ phase: "committing" }))).toBeNull();
});

test("maps stage labels consistently", () => {
  expect(getIndexProgressStageLabel(progress({ phase: "search" }))).toBe("Stage 6/6");
});
