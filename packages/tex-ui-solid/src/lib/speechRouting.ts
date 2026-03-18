import type { TexBlock, TexRouteHeading, TexSendInsertMode } from "./types";

export type SpeechSendSource = {
  sourceBlocks: TexBlock[];
  sourceRootLevel: number;
  sourceMaxRelativeDepth: number;
  sourceHeadingText: string;
};

export type SpeechPlacement = {
  allowed: boolean;
  reason: string | null;
};

export const buildSpeechSendSource = (
  blocks: TexBlock[],
  activeBlockIndex: number | null,
): SpeechSendSource | { error: string } => {
  if (activeBlockIndex == null || activeBlockIndex < 0 || activeBlockIndex >= blocks.length) {
    return { error: "Move the caret onto a Pocket, Hat, Block, or Tag heading to send." };
  }

  const activeBlock = blocks[activeBlockIndex];
  if (
    !activeBlock ||
    activeBlock.kind !== "heading" ||
    activeBlock.level == null ||
    activeBlock.level < 1 ||
    activeBlock.level > 4
  ) {
    return { error: "Move the caret onto a Pocket, Hat, Block, or Tag heading to send." };
  }

  const sourceRootLevel = activeBlock.level;
  let endIndex = blocks.length;
  for (let index = activeBlockIndex + 1; index < blocks.length; index += 1) {
    const candidate = blocks[index];
    if (
      candidate?.kind === "heading" &&
      candidate.level != null &&
      candidate.level <= sourceRootLevel
    ) {
      endIndex = index;
      break;
    }
  }

  const sourceBlocks = blocks.slice(activeBlockIndex, endIndex);
  let sourceMaxRelativeDepth = 0;
  for (const block of sourceBlocks) {
    if (block.kind !== "heading" || block.level == null) {
      continue;
    }
    sourceMaxRelativeDepth = Math.max(sourceMaxRelativeDepth, block.level - sourceRootLevel);
  }

  return {
    sourceBlocks,
    sourceRootLevel,
    sourceMaxRelativeDepth,
    sourceHeadingText: activeBlock.text || "(empty)",
  };
};

export const placementForTarget = (
  target: TexRouteHeading | null,
  insertMode: TexSendInsertMode,
  sourceMaxRelativeDepth: number,
): SpeechPlacement => {
  if (!target) {
    return insertMode === "below"
      ? { allowed: true, reason: null }
      : { allowed: false, reason: "Document Root only supports Below." };
  }

  if (insertMode === "under") {
    if (target.level >= 4) {
      return {
        allowed: false,
        reason: "Under is unavailable here because this send would exceed Tag depth.",
      };
    }
    if (target.level + 1 + sourceMaxRelativeDepth > 4) {
      return {
        allowed: false,
        reason: "Under is unavailable here because this send would exceed Tag depth.",
      };
    }
    return { allowed: true, reason: null };
  }

  if (target.level + sourceMaxRelativeDepth > 4) {
    return {
      allowed: false,
      reason: "Below is unavailable here because this send would exceed Tag depth.",
    };
  }

  return { allowed: true, reason: null };
};
