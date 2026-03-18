import { setBlockType, toggleMark } from "prosemirror-commands";
import { TextSelection, type Selection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { texEditorSchema } from "../editor-schema";

const TEXT_MARK_NAMES = ["strong", "em", "underline", "highlight", "cite"] as const;

export const isMarkActive = (
  view: EditorView,
  markName: "strong" | "underline" | "highlight" | "cite",
) => {
  const markType = texEditorSchema.marks[markName];
  const { from, to, empty } = view.state.selection;

  if (empty) {
    const stored = view.state.storedMarks ?? view.state.selection.$from.marks();
    return !!markType.isInSet(stored);
  }

  let active = false;
  view.state.doc.nodesBetween(from, to, (node) => {
    if (active || !node.isText) {
      return;
    }
    active = !!markType.isInSet(node.marks);
  });
  return active;
};

export const isHeadingActive = (view: EditorView, level: number) => {
  const parent = view.state.selection.$from.parent;
  return parent.type.name === "heading" && Number(parent.attrs.level ?? 1) === level;
};

export const isCiteActive = (view: EditorView) =>
  isMarkActive(view, "cite") ||
  view.state.selection.$from.parent.attrs.styleId === "Cite" ||
  !!view.state.selection.$from.parent.attrs.isF8Cite;

const restoreTextCursor = (view: EditorView, previousSelection: Selection) => {
  const { selection, doc } = view.state;
  const anchor = Math.max(0, Math.min(previousSelection.head, doc.content.size));
  const nextSelection = TextSelection.create(doc, anchor);
  if (nextSelection.from !== selection.from || nextSelection.to !== selection.to) {
    view.dispatch(view.state.tr.setSelection(nextSelection));
  }
};

export const setHeadingLevel = (view: EditorView, level: number) => {
  const previousSelection = view.state.selection;
  const command = setBlockType(texEditorSchema.nodes.heading, {
    level,
    styleId: `Heading${level}`,
    styleName: `Heading ${level}`,
    isF8Cite: false,
  });
  if (command(view.state, view.dispatch, view)) {
    restoreTextCursor(view, previousSelection);
  }
  view.focus();
};

const setParagraphStyle = (
  view: EditorView,
  styleId = "Normal",
  styleName = "Normal",
  isF8Cite = false,
) => {
  const previousSelection = view.state.selection;
  const command = setBlockType(texEditorSchema.nodes.paragraph, {
    styleId,
    styleName,
    isF8Cite,
  });
  if (command(view.state, view.dispatch, view)) {
    restoreTextCursor(view, previousSelection);
  }
  view.focus();
};

export const toggleCiteStyle = (view: EditorView) => {
  toggleMark(texEditorSchema.marks.cite, {
    styleId: "Cite",
    styleName: "Cite",
  })(view.state, view.dispatch, view);
  view.focus();
};

export const toggleHighlight = (view: EditorView) => {
  const command = toggleMark(texEditorSchema.marks.highlight, { color: "blue" });
  command(view.state, view.dispatch, view);
  view.focus();
};

const clearInlineFormatting = (view: EditorView, from: number, to: number) => {
  const transaction = view.state.tr;
  for (const markName of TEXT_MARK_NAMES) {
    transaction.removeMark(from, to, texEditorSchema.marks[markName]);
  }
  view.dispatch(transaction);
};

export const toggleUnderlineStyle = (view: EditorView) => {
  const { empty, from, to } = view.state.selection;
  if (isMarkActive(view, "underline")) {
    if (empty) {
      view.dispatch(view.state.tr.setStoredMarks([]));
      view.focus();
      return;
    }

    clearInlineFormatting(view, from, to);
    view.focus();
    return;
  }

  toggleMark(texEditorSchema.marks.underline)(view.state, view.dispatch, view);
  view.focus();
};

export const clearToNormal = (view: EditorView) => {
  const { empty, from, to } = view.state.selection;

  if (!empty) {
    clearInlineFormatting(view, from, to);
    view.focus();
    return;
  }

  setParagraphStyle(view, "Normal", "Normal", false);
};

export const toggleStrong = (view: EditorView) => {
  toggleMark(texEditorSchema.marks.strong)(view.state, view.dispatch, view);
  view.focus();
};
