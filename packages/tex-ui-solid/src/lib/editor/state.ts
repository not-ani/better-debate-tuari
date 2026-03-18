import { baseKeymap } from "prosemirror-commands";
import { history, redo, undo } from "prosemirror-history";
import { keymap } from "prosemirror-keymap";
import { EditorState, Plugin, TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { texBlocksToPmDoc, texEditorSchema } from "../editor-schema";
import {
  createStickyHighlightMark,
  getStoredMarksWithoutStickyHighlight,
  getStickyHighlightSelectionAction,
  STICKY_HIGHLIGHT_AUTO_APPLY_META,
  STICKY_HIGHLIGHT_CLEAR_META,
  STICKY_HIGHLIGHT_TRIGGER_META,
} from "./sticky-highlight";
import type { TexDocumentPayload } from "../types";

type FunctionKeyHandler = (key: string) => boolean;

const stickyHighlightPlugin = (isStickyHighlightEnabled: () => boolean) =>
  new Plugin({
    props: {
      handleDOMEvents: {
        mouseup(view) {
          if (!isStickyHighlightEnabled()) {
            return false;
          }

          queueMicrotask(() => {
            view.dispatch(view.state.tr.setMeta(STICKY_HIGHLIGHT_TRIGGER_META, true));
          });
          return false;
        },
        keyup(view, event) {
          if (
            !isStickyHighlightEnabled() ||
            !(event.key.startsWith("Arrow") || event.key === "Shift")
          ) {
            return false;
          }

          queueMicrotask(() => {
            view.dispatch(view.state.tr.setMeta(STICKY_HIGHLIGHT_TRIGGER_META, true));
          });
          return false;
        },
      },
    },
    appendTransaction(transactions, _oldState, newState) {
      if (!isStickyHighlightEnabled()) {
        return null;
      }

      if (
        transactions.some(
          (transaction) =>
            transaction.getMeta(STICKY_HIGHLIGHT_CLEAR_META),
        )
      ) {
        return null;
      }

      const triggered = transactions.some((transaction) =>
        transaction.getMeta(STICKY_HIGHLIGHT_TRIGGER_META),
      );
      if (!triggered) {
        return null;
      }

      if (!newState.selection.empty) {
        const { from, to } = newState.selection;
        const action = getStickyHighlightSelectionAction(newState);
        if (action === "add" || action === "remove") {
          const nextTransaction = newState.tr;
          if (action === "add") {
            nextTransaction.addMark(from, to, createStickyHighlightMark());
          } else {
            nextTransaction.removeMark(from, to, texEditorSchema.marks.highlight);
          }

          const collapsedSelection = TextSelection.create(nextTransaction.doc, to);
          const previewState = newState.apply(nextTransaction.setSelection(collapsedSelection));
          const clearedStoredMarks = getStoredMarksWithoutStickyHighlight(previewState);

          nextTransaction.setMeta(STICKY_HIGHLIGHT_AUTO_APPLY_META, true);
          if (clearedStoredMarks) {
            nextTransaction.setStoredMarks(clearedStoredMarks);
          }

          return nextTransaction;
        }
      }

      if (newState.selection.empty) {
        const clearedStoredMarks = getStoredMarksWithoutStickyHighlight(newState);
        if (clearedStoredMarks) {
          return newState.tr
            .setStoredMarks(clearedStoredMarks)
            .setMeta(STICKY_HIGHLIGHT_CLEAR_META, true);
        }
      }

      return null;
    },
  });

export const createTexEditorState = (
  document: TexDocumentPayload,
  onSave: () => void,
  runFunctionKey: FunctionKeyHandler,
  isStickyHighlightEnabled: () => boolean,
) =>
  EditorState.create({
    schema: texEditorSchema,
    doc: texBlocksToPmDoc(document.blocks),
    plugins: [
      history(),
      keymap({
        F2: () => runFunctionKey("F2"),
        F3: () => runFunctionKey("F3"),
        F4: () => runFunctionKey("F4"),
        F5: () => runFunctionKey("F5"),
        F6: () => runFunctionKey("F6"),
        F7: () => runFunctionKey("F7"),
        F8: () => runFunctionKey("F8"),
        F9: () => runFunctionKey("F9"),
        F10: () => runFunctionKey("F10"),
        F11: () => runFunctionKey("F11"),
        F12: () => runFunctionKey("F12"),
        "Mod-z": undo,
        "Shift-Mod-z": redo,
        "Mod-y": redo,
        "Mod-s": () => {
          onSave();
          return true;
        },
      }),
      keymap(baseKeymap),
      stickyHighlightPlugin(isStickyHighlightEnabled),
    ],
  });

export const scrollToBlock = (view: EditorView, blockIndex: number) => {
  const doc = view.state.doc;
  if (blockIndex < 0 || blockIndex >= doc.childCount) {
    return;
  }

  let pos = 0;
  for (let index = 0; index < blockIndex; index += 1) {
    pos += doc.child(index).nodeSize;
  }
  pos += 1;

  view.dispatch(view.state.tr.setSelection(TextSelection.create(doc, pos)).scrollIntoView());
  view.focus();
};
