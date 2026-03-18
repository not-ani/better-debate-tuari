import { Fragment, Node as PMNode } from "prosemirror-model";
import { TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { pmDocToTexBlocks, texBlocksToPmDoc } from "../editor-schema";
import type { TexBlock } from "../types";

const normalizeWhitespace = (value: string) => value.replace(/\s+/g, " ").trim();

const isCiteLikeBlock = (block: TexBlock) => {
  const trimmed = block.text.trim();
  if (!trimmed) {
    return false;
  }

  const lowered = trimmed.toLowerCase();
  return (
    block.isF8Cite ||
    block.styleId === "Cite" ||
    block.styleName === "Cite" ||
    block.runs.some((run) => run.isF8Cite) ||
    /^[\[(<*]/.test(trimmed) ||
    lowered.includes("http://") ||
    lowered.includes("https://") ||
    /\b(omitted|edited|modified|sic)\b/.test(lowered)
  );
};

const findBlockIndexAtPosition = (doc: PMNode, pos: number) => {
  let cursor = 0;
  for (let index = 0; index < doc.childCount; index += 1) {
    const child = doc.child(index);
    const start = cursor + 1;
    const end = cursor + child.nodeSize;
    if (pos >= start && pos <= end) {
      return index;
    }
    cursor = end;
  }
  return Math.max(0, doc.childCount - 1);
};

const replaceEntireDocument = (view: EditorView, blocks: TexBlock[]) => {
  const nextDoc = texBlocksToPmDoc(blocks);
  const transaction = view.state.tr.replaceWith(
    0,
    view.state.doc.content.size,
    Fragment.from(nextDoc.content),
  );
  transaction.setSelection(
    TextSelection.create(transaction.doc, Math.min(1, transaction.doc.content.size)),
  );
  view.dispatch(transaction);
  view.focus();
};

const toPlainRun = (text: string) => ({
  text,
  bold: false,
  italic: false,
  underline: false,
  smallCaps: false,
  highlightColor: null,
  styleId: null,
  styleName: null,
  isF8Cite: false,
});

const condenseBlockRange = (blocks: TexBlock[], bodyStart: number, sectionEnd: number) => {
  if (bodyStart >= sectionEnd) {
    return blocks;
  }

  const bodyBlocks = blocks.slice(bodyStart, sectionEnd);
  const condensedText = normalizeWhitespace(bodyBlocks.map((block) => block.text).join(" "));
  if (!condensedText) {
    return blocks;
  }

  const firstBody = bodyBlocks[0]!;
  const condensedBlock: TexBlock = {
    ...firstBody,
    text: condensedText,
    runs: [toPlainRun(condensedText)],
    styleId: firstBody.styleId ?? "Normal",
    styleName: firstBody.styleName ?? "Normal",
    isF8Cite: false,
  };

  return [...blocks.slice(0, bodyStart), condensedBlock, ...blocks.slice(sectionEnd)];
};

const condenseCurrentCard = (view: EditorView, blocks: TexBlock[], from: number) => {
  const activeIndex = findBlockIndexAtPosition(view.state.doc, from);
  let tagIndex = -1;
  for (let index = activeIndex; index >= 0; index -= 1) {
    const block = blocks[index];
    if (block?.kind === "heading" && block.level === 4) {
      tagIndex = index;
      break;
    }
  }

  if (tagIndex === -1) {
    const block = blocks[activeIndex]!;
    const condensedText = normalizeWhitespace(block.text);
    if (!condensedText || condensedText === block.text) {
      return blocks;
    }

    return [
      ...blocks.slice(0, activeIndex),
      {
        ...block,
        text: condensedText,
        runs: [toPlainRun(condensedText)],
      },
      ...blocks.slice(activeIndex + 1),
    ];
  }

  let sectionEnd = blocks.length;
  for (let index = tagIndex + 1; index < blocks.length; index += 1) {
    const block = blocks[index];
    if (block?.kind === "heading" && (block.level ?? 9) <= 4) {
      sectionEnd = index;
      break;
    }
  }

  let bodyStart = tagIndex + 1;
  while (bodyStart < sectionEnd && isCiteLikeBlock(blocks[bodyStart]!)) {
    bodyStart += 1;
  }

  return condenseBlockRange(blocks, bodyStart, sectionEnd);
};

const condenseAllCards = (blocks: TexBlock[]) => {
  let nextBlocks = blocks.slice();
  let index = 0;

  while (index < nextBlocks.length) {
    const block = nextBlocks[index];
    if (!(block?.kind === "heading" && block.level === 4)) {
      index += 1;
      continue;
    }

    let sectionEnd = nextBlocks.length;
    for (let pointer = index + 1; pointer < nextBlocks.length; pointer += 1) {
      const candidate = nextBlocks[pointer];
      if (candidate?.kind === "heading" && (candidate.level ?? 9) <= 4) {
        sectionEnd = pointer;
        break;
      }
    }

    let bodyStart = index + 1;
    while (bodyStart < sectionEnd && isCiteLikeBlock(nextBlocks[bodyStart]!)) {
      bodyStart += 1;
    }

    nextBlocks = condenseBlockRange(nextBlocks, bodyStart, sectionEnd);
    index = bodyStart + 1;
  }

  return nextBlocks;
};

export const condenseAllOrSelection = (view: EditorView) => {
  const { empty, from, to } = view.state.selection;

  if (!empty) {
    const condensed = normalizeWhitespace(view.state.doc.textBetween(from, to, " ", " "));
    if (condensed) {
      view.dispatch(view.state.tr.insertText(condensed, from, to));
      view.focus();
    }
    return;
  }

  const blocks = pmDocToTexBlocks(view.state.doc);
  if (blocks.length === 0) {
    return;
  }

  const nextBlocks = from <= 1 ? condenseAllCards(blocks) : condenseCurrentCard(view, blocks, from);
  if (JSON.stringify(nextBlocks) !== JSON.stringify(blocks)) {
    replaceEntireDocument(view, nextBlocks);
  }
};

export const pastePlainText = async (view: EditorView) => {
  try {
    const text = await navigator.clipboard.readText();
    if (!text) {
      return;
    }
    const { from, to } = view.state.selection;
    view.dispatch(view.state.tr.insertText(text, from, to));
    view.focus();
  } catch {
    view.focus();
  }
};
