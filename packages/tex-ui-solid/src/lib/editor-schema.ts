import { Mark, Node as PMNode, Schema, type DOMOutputSpec } from "prosemirror-model";
import type { TexBlock, TexTextRun } from "./types";

const getBlockAttrs = (element: string | HTMLElement) => {
  const block = element as HTMLElement;
  return {
    styleId: block.dataset.styleId ?? "Normal",
    styleName: block.dataset.styleName ?? block.dataset.styleId ?? "Normal",
    isF8Cite: block.dataset.f8Cite === "true",
  };
};

const nodes = {
  doc: {
    content: "block+",
  },
  paragraph: {
    group: "block",
    content: "inline*",
    attrs: {
      styleId: { default: "Normal" },
      styleName: { default: "Normal" },
      isF8Cite: { default: false },
    },
    parseDOM: [
      {
        tag: "p",
        getAttrs: getBlockAttrs,
      },
    ],
    toDOM(node: PMNode): DOMOutputSpec {
      return [
        "p",
        {
          "data-style-id": node.attrs.styleId ?? "",
          "data-style-name": node.attrs.styleName ?? "",
          "data-f8-cite": node.attrs.isF8Cite ? "true" : "false",
        },
        0,
      ] as const;
    },
  },
  heading: {
    group: "block",
    content: "inline*",
    defining: true,
    attrs: {
      level: { default: 1 },
      styleId: { default: null },
      styleName: { default: null },
      isF8Cite: { default: false },
    },
    parseDOM: [1, 2, 3, 4].map((level) => ({
      tag: `h${level}`,
      getAttrs: (element: string | HTMLElement) => ({
        level,
        ...getBlockAttrs(element),
      }),
    })),
    toDOM(node: PMNode): DOMOutputSpec {
      const level = Math.max(1, Math.min(4, Number(node.attrs.level) || 1));
      return [
        `h${level}`,
        {
          "data-style-id": node.attrs.styleId ?? "",
          "data-style-name": node.attrs.styleName ?? "",
          "data-f8-cite": node.attrs.isF8Cite ? "true" : "false",
        },
        0,
      ] as const;
    },
  },
  text: {
    group: "inline",
  },
  hard_break: {
    group: "inline",
    inline: true,
    selectable: false,
    parseDOM: [{ tag: "br" }],
    toDOM(): DOMOutputSpec {
      return ["br"] as const;
    },
  },
};

const marks = {
  strong: {
    parseDOM: [{ tag: "strong" }, { tag: "b" }],
    toDOM(): DOMOutputSpec {
      return ["strong", 0] as const;
    },
  },
  em: {
    parseDOM: [{ tag: "em" }, { tag: "i" }],
    toDOM(): DOMOutputSpec {
      return ["em", 0] as const;
    },
  },
  underline: {
    parseDOM: [{ tag: "u" }],
    toDOM(): DOMOutputSpec {
      return ["u", 0] as const;
    },
  },
  cite: {
    attrs: {
      styleId: { default: "Cite" },
      styleName: { default: "Cite" },
    },
    parseDOM: [
      {
        tag: 'span[data-tex-cite="true"]',
        getAttrs: (element: string | HTMLElement) => ({
          styleId: (element as HTMLElement).dataset.styleId ?? "Cite",
          styleName: (element as HTMLElement).dataset.styleName ?? "Cite",
        }),
      },
    ],
    toDOM(mark: Mark): DOMOutputSpec {
      return [
        "span",
        {
          "data-tex-cite": "true",
          "data-style-id": mark.attrs.styleId ?? "Cite",
          "data-style-name": mark.attrs.styleName ?? "Cite",
        },
        0,
      ] as const;
    },
  },
  highlight: {
    attrs: {
      color: { default: "blue" },
    },
    parseDOM: [
      {
        tag: "mark",
        getAttrs: (element: string | HTMLElement) => ({
          color: (element as HTMLElement).dataset.color ?? "blue",
        }),
      },
    ],
    toDOM(mark: Mark): DOMOutputSpec {
      return ["mark", { "data-color": mark.attrs.color ?? "blue" }, 0] as const;
    },
  },
};

export const texEditorSchema = new Schema({ nodes, marks });

const sameFormatting = (left: TexTextRun, right: TexTextRun) =>
  left.bold === right.bold &&
  left.italic === right.italic &&
  left.underline === right.underline &&
  left.smallCaps === right.smallCaps &&
  left.highlightColor === right.highlightColor &&
  left.styleId === right.styleId &&
  left.styleName === right.styleName &&
  left.isF8Cite === right.isF8Cite;

const buildMarks = (run: TexTextRun) => {
  const schema = texEditorSchema;
  const result: Mark[] = [];
  if (run.bold) {
    result.push(schema.marks.strong.create());
  }
  if (run.italic) {
    result.push(schema.marks.em.create());
  }
  if (run.underline) {
    result.push(schema.marks.underline.create());
  }
  if (run.isF8Cite) {
    result.push(
      schema.marks.cite.create({
        styleId: run.styleId ?? "Cite",
        styleName: run.styleName ?? "Cite",
      }),
    );
  }
  if (run.highlightColor) {
    result.push(schema.marks.highlight.create({ color: run.highlightColor }));
  }
  return result;
};

const appendTextNode = (children: PMNode[], text: string, marksForRun: Mark[]) => {
  if (!text) {
    return;
  }
  children.push(texEditorSchema.text(text, marksForRun));
};

const runToChildren = (run: TexTextRun) => {
  const children: PMNode[] = [];
  const parts = run.text.split("\n");
  const marksForRun = buildMarks(run);

  parts.forEach((part, index) => {
    appendTextNode(children, part, marksForRun);
    if (index < parts.length - 1) {
      children.push(texEditorSchema.nodes.hard_break.create());
    }
  });

  return children;
};

export const texBlocksToPmDoc = (blocks: TexBlock[]) => {
  const normalizedBlocks = blocks.length > 0
    ? blocks
    : [
        {
          id: "empty-1",
          kind: "paragraph",
          text: "",
          runs: [],
          level: null,
          styleId: "Normal",
          styleName: "Normal",
          isF8Cite: false,
        } satisfies TexBlock,
      ];

  return texEditorSchema.nodes.doc.create(
    null,
    normalizedBlocks.map((block) => {
      const content = block.runs.flatMap(runToChildren);
      if (block.kind === "heading") {
        return texEditorSchema.nodes.heading.create(
          {
            level: block.level ?? 1,
            styleId: block.styleId ?? `Heading${block.level ?? 1}`,
            styleName: block.styleName ?? `Heading ${block.level ?? 1}`,
            isF8Cite: block.isF8Cite,
          },
          content,
        );
      }

      return texEditorSchema.nodes.paragraph.create(
        {
          styleId: block.styleId ?? "Normal",
          styleName: block.styleName ?? "Normal",
          isF8Cite: block.isF8Cite,
        },
        content,
      );
    }),
  );
};

const blockRunFromMarks = (text: string, marksForNode: readonly Mark[]): TexTextRun => {
  const highlightMark = marksForNode.find((mark) => mark.type.name === "highlight");
  const citeMark = marksForNode.find((mark) => mark.type.name === "cite");
  return {
    text,
    bold: marksForNode.some((mark) => mark.type.name === "strong"),
    italic: marksForNode.some((mark) => mark.type.name === "em"),
    underline: marksForNode.some((mark) => mark.type.name === "underline"),
    smallCaps: false,
    highlightColor: typeof highlightMark?.attrs.color === "string" ? highlightMark.attrs.color : null,
    styleId: typeof citeMark?.attrs.styleId === "string" ? citeMark.attrs.styleId : null,
    styleName: typeof citeMark?.attrs.styleName === "string" ? citeMark.attrs.styleName : null,
    isF8Cite: !!citeMark,
  };
};

const pushRun = (runs: TexTextRun[], nextRun: TexTextRun) => {
  if (!nextRun.text) {
    return;
  }

  const previous = runs[runs.length - 1];
  if (previous && sameFormatting(previous, nextRun)) {
    previous.text += nextRun.text;
    return;
  }

  runs.push(nextRun);
};

export const pmDocToTexBlocks = (doc: PMNode): TexBlock[] => {
  const blocks: TexBlock[] = [];

  doc.forEach((blockNode, offset, index) => {
    const runs: TexTextRun[] = [];

    blockNode.forEach((child) => {
      if (child.isText) {
        pushRun(runs, blockRunFromMarks(child.text ?? "", child.marks));
        return;
      }

      if (child.type.name === "hard_break") {
        if (runs.length === 0) {
          runs.push({
            text: "\n",
            bold: false,
            italic: false,
            underline: false,
            smallCaps: false,
            highlightColor: null,
            styleId: null,
            styleName: null,
            isF8Cite: false,
          });
          return;
        }

        runs[runs.length - 1].text += "\n";
      }
    });

    const text = runs.map((run) => run.text).join("");
    const isHeading = blockNode.type.name === "heading";
    const level = isHeading ? Number(blockNode.attrs.level ?? 1) : null;

    blocks.push({
      id: `pm-${index}-${offset}`,
      kind: isHeading ? "heading" : "paragraph",
      text,
      runs,
      level,
      styleId:
        typeof blockNode.attrs.styleId === "string" && blockNode.attrs.styleId.length > 0
          ? blockNode.attrs.styleId
          : isHeading
            ? `Heading${level ?? 1}`
            : "Normal",
      styleName:
        typeof blockNode.attrs.styleName === "string" && blockNode.attrs.styleName.length > 0
          ? blockNode.attrs.styleName
          : isHeading
            ? `Heading ${level ?? 1}`
            : "Normal",
      isF8Cite: !!blockNode.attrs.isF8Cite,
    });
  });

  return blocks;
};
