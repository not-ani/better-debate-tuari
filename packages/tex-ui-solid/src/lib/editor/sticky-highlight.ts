import type { Mark } from "prosemirror-model";
import type { EditorState } from "prosemirror-state";
import { texEditorSchema } from "../editor-schema";

export const STICKY_HIGHLIGHT_COLOR = "blue";
export const STICKY_HIGHLIGHT_TRIGGER_META = "stickyHighlightTrigger";
export const STICKY_HIGHLIGHT_AUTO_APPLY_META = "stickyHighlightAutoApply";
export const STICKY_HIGHLIGHT_CLEAR_META = "stickyHighlightClearStoredMark";

const highlightType = texEditorSchema.marks.highlight;

export type StickyHighlightSelectionAction = "add" | "remove" | null;

export const createStickyHighlightMark = () =>
  highlightType.create({ color: STICKY_HIGHLIGHT_COLOR });

export const isStickyHighlightMark = (mark: Mark | null | undefined) =>
  !!mark &&
  mark.type === highlightType &&
  typeof mark.attrs.color === "string" &&
  mark.attrs.color === STICKY_HIGHLIGHT_COLOR;

export const getStickyHighlightSelectionAction = (
  state: EditorState,
): StickyHighlightSelectionAction => {
  const { empty, from, to } = state.selection;
  if (empty) {
    return null;
  }

  let hasHighlightedText = false;
  let hasUnhighlightedText = false;
  state.doc.nodesBetween(from, to, (node) => {
    if (!node.isText || !node.text) {
      return;
    }

    const highlightMark = highlightType.isInSet(node.marks);
    if (isStickyHighlightMark(highlightMark)) {
      hasHighlightedText = true;
    } else {
      hasUnhighlightedText = true;
    }
  });

  if (!hasHighlightedText && hasUnhighlightedText) {
    return "add";
  }

  if (hasHighlightedText && !hasUnhighlightedText) {
    return "remove";
  }

  if (hasHighlightedText && hasUnhighlightedText) {
    return "add";
  }

  return null;
};

export const getStoredMarksWithoutStickyHighlight = (state: EditorState) => {
  const marks = state.storedMarks ?? state.selection.$from.marks();
  const hasStickyHighlight = marks.some((mark) => isStickyHighlightMark(mark));

  if (!hasStickyHighlight) {
    return null;
  }

  return marks.filter((mark) => !isStickyHighlightMark(mark));
};
