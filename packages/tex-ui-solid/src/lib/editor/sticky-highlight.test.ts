import { expect, test } from "bun:test";
import { EditorState, TextSelection } from "prosemirror-state";
import { texEditorSchema } from "../editor-schema";
import {
  createStickyHighlightMark,
  getStickyHighlightSelectionAction,
} from "./sticky-highlight";

const createParagraphState = ({
  textNodes,
  from,
  to,
}: {
  textNodes: ReturnType<typeof texEditorSchema.text>[];
  from: number;
  to: number;
}) => {
  const paragraph = texEditorSchema.nodes.paragraph.create(null, textNodes);
  const doc = texEditorSchema.nodes.doc.create(null, [paragraph]);
  return EditorState.create({
    schema: texEditorSchema,
    doc,
    selection: TextSelection.create(doc, from, to),
  });
};

test("getStickyHighlightSelectionAction returns null for empty selections", () => {
  const state = createParagraphState({
    textNodes: [texEditorSchema.text("hello world")],
    from: 1,
    to: 1,
  });

  expect(getStickyHighlightSelectionAction(state)).toBeNull();
});

test('getStickyHighlightSelectionAction returns "add" for unhighlighted selections', () => {
  const state = createParagraphState({
    textNodes: [texEditorSchema.text("hello world")],
    from: 1,
    to: 6,
  });

  expect(getStickyHighlightSelectionAction(state)).toBe("add");
});

test('getStickyHighlightSelectionAction returns "remove" when selection is fully highlighted', () => {
  const highlightMark = createStickyHighlightMark();
  const state = createParagraphState({
    textNodes: [texEditorSchema.text("hello world", [highlightMark])],
    from: 1,
    to: 6,
  });

  expect(getStickyHighlightSelectionAction(state)).toBe("remove");
});

test('getStickyHighlightSelectionAction returns "add" when selection is partially highlighted', () => {
  const highlightMark = createStickyHighlightMark();
  const state = createParagraphState({
    textNodes: [
      texEditorSchema.text("hello", [highlightMark]),
      texEditorSchema.text(" world"),
    ],
    from: 1,
    to: 12,
  });

  expect(getStickyHighlightSelectionAction(state)).toBe("add");
});
