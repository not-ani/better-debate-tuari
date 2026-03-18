import { expect, test } from "bun:test";
import type { TexBlock } from "./types";
import { buildSpeechSendSource, placementForTarget } from "./speechRouting";

const heading = (level: number, text: string): TexBlock => ({
  id: `h-${level}-${text}`,
  kind: "heading",
  text,
  runs: [],
  level,
  styleId: `Heading${level}`,
  styleName: `Heading ${level}`,
  isF8Cite: false,
});

const paragraph = (text: string): TexBlock => ({
  id: `p-${text}`,
  kind: "paragraph",
  text,
  runs: [],
  level: null,
  styleId: null,
  styleName: null,
  isF8Cite: false,
});

test("buildSpeechSendSource returns full pocket subtree", () => {
  const blocks = [
    heading(1, "Pocket"),
    heading(2, "Hat"),
    heading(4, "Tag"),
    paragraph("card body"),
    heading(1, "Next pocket"),
  ];

  const result = buildSpeechSendSource(blocks, 0);
  if ("error" in result) throw new Error(result.error);

  expect(result.sourceBlocks).toHaveLength(4);
  expect(result.sourceRootLevel).toBe(1);
  expect(result.sourceMaxRelativeDepth).toBe(3);
});

test("buildSpeechSendSource returns tag plus body only", () => {
  const blocks = [
    heading(3, "Block"),
    heading(4, "Tag"),
    paragraph("card body"),
    heading(4, "Next tag"),
  ];

  const result = buildSpeechSendSource(blocks, 1);
  if ("error" in result) throw new Error(result.error);

  expect(result.sourceBlocks).toHaveLength(2);
  expect(result.sourceRootLevel).toBe(4);
});

test("buildSpeechSendSource rejects body paragraph caret", () => {
  const blocks = [heading(4, "Tag"), paragraph("card body")];
  const result = buildSpeechSendSource(blocks, 1);
  expect("error" in result).toBe(true);
});

test("placementForTarget rejects under tag", () => {
  const result = placementForTarget({ blockIndex: 3, level: 4, text: "Tag" }, "under", 0);
  expect(result.allowed).toBe(false);
});

test("placementForTarget allows below document root", () => {
  const result = placementForTarget(null, "below", 2);
  expect(result.allowed).toBe(true);
});
