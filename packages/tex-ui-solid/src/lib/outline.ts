import type { TexBlock } from "./types";

export type OutlineNode = {
  text: string;
  level: number;
  blockIndex: number;
  children: OutlineNode[];
};

export const HEADING_LABELS: Record<number, string> = {
  1: "Pocket",
  2: "Hat",
  3: "Block",
  4: "Tag",
};

export const buildOutlineTree = (blocks: TexBlock[]): OutlineNode[] => {
  const headings: { text: string; level: number; blockIndex: number }[] = [];
  for (let index = 0; index < blocks.length; index += 1) {
    const block = blocks[index]!;
    if (block.kind === "heading" && block.level != null && block.level >= 1 && block.level <= 4) {
      headings.push({ text: block.text || "(empty)", level: block.level, blockIndex: index });
    }
  }

  const root: OutlineNode[] = [];
  const stack: { node: OutlineNode; level: number }[] = [];

  for (const heading of headings) {
    const node: OutlineNode = {
      text: heading.text,
      level: heading.level,
      blockIndex: heading.blockIndex,
      children: [],
    };

    while (stack.length > 0 && stack[stack.length - 1]!.level >= heading.level) {
      stack.pop();
    }

    if (stack.length === 0) {
      root.push(node);
    } else {
      stack[stack.length - 1]!.node.children.push(node);
    }

    stack.push({ node, level: heading.level });
  }

  return root;
};
