export type RecentFile = {
  path: string;
  name: string;
  openedAtMs: number;
};

export type TexTextRun = {
  text: string;
  bold: boolean;
  italic: boolean;
  underline: boolean;
  smallCaps: boolean;
  highlightColor: string | null;
  styleId: string | null;
  styleName: string | null;
  isF8Cite: boolean;
};

export type TexBlock = {
  id: string;
  kind: string;
  text: string;
  runs: TexTextRun[];
  level: number | null;
  styleId: string | null;
  styleName: string | null;
  isF8Cite: boolean;
};

export type TexDocumentPayload = {
  filePath: string;
  fileName: string;
  paragraphCount: number;
  blocks: TexBlock[];
};

export type TexSessionSnapshot = TexDocumentPayload & {
  sessionId: string;
  version: number;
  dirty: boolean;
};

export type TexSessionSummary = {
  sessionId: string;
  filePath: string;
  fileName: string;
  version: number;
  dirty: boolean;
  paragraphCount: number;
  updatedAtMs: number;
};

export type TexRecoverableSession = TexSessionSummary;

export type TexRouteHeading = {
  blockIndex: number;
  level: number;
  text: string;
};

export type TexSessionRouteTarget = {
  sessionId: string;
  filePath: string;
  fileName: string;
  ownerWindowLabel: string | null;
  dirty: boolean;
  headings: TexRouteHeading[];
};

export type TexSpeechTargetState = {
  targetSessionId: string | null;
};

export type TexSendInsertMode = "below" | "under";

export type TexSendRequest = {
  targetSessionId: string;
  targetBlockIndex: number | null;
  insertMode: TexSendInsertMode;
  sourceBlocks: TexBlock[];
  sourceRootLevel: number;
  sourceMaxRelativeDepth: number;
};

export type TexSendResult = {
  targetSessionId: string;
  snapshot: TexSessionSnapshot;
};

export type TexSessionUpdatedEvent = {
  snapshot: TexSessionSnapshot;
};

export type TexSessionOwnerConflict = {
  kind: "ownerConflict";
  sessionId: string;
  filePath: string;
  fileName: string;
  ownerWindowLabel: string;
};

export type TexSessionOpenResult =
  | {
      kind: "opened";
      snapshot: TexSessionSnapshot;
    }
  | TexSessionOwnerConflict;

export type TexSessionAttachResult =
  | {
      kind: "attached";
      snapshot: TexSessionSnapshot;
    }
  | TexSessionOwnerConflict;

export type TexSessionUpdateArgs = {
  sessionId: string;
  windowLabel: string;
  baseVersion: number;
  document: TexDocumentPayload;
};

export type OpenDocumentResult = {
  document: TexDocumentPayload;
  recentFiles: RecentFile[];
};
