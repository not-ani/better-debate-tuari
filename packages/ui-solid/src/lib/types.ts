export type RootSummary = {
  path: string;
  fileCount: number;
  headingCount: number;
  addedAtMs: number;
  lastIndexedMs: number;
};

export type RootIndexEntry = {
  rootId: number;
  rootPath: string;
  folderName: string;
  indexPath: string;
  indexSizeBytes: number;
  fileCount: number;
  headingCount: number;
  lastIndexedMs: number;
};

export type FolderEntry = {
  path: string;
  name: string;
  parentPath: string | null;
  depth: number;
  fileCount: number;
};

export type IndexedFile = {
  id: number;
  fileName: string;
  relativePath: string;
  folderPath: string;
  modifiedMs: number;
  headingCount: number;
};

export type IndexSnapshot = {
  rootPath: string;
  indexedAtMs: number;
  folders: FolderEntry[];
  files: IndexedFile[];
};

export type FileHeading = {
  id: number;
  order: number;
  level: number;
  text: string;
  copyText: string;
};

export type TaggedBlock = {
  order: number;
  styleLabel: string;
  text: string;
};

export type FilePreview = {
  fileId: number;
  fileName: string;
  relativePath: string;
  absolutePath: string;
  headingCount: number;
  headings: FileHeading[];
  f8Cites: TaggedBlock[];
};

export type HighlightSpan = {
  field: string;
  start: number;
  end: number;
};

export type SearchHit = {
  resultId: number;
  entityType: "doc" | "card";
  rootPath: string;
  source: "lexical" | "mixed";
  kind: "heading" | "file" | "author";
  fileId: number;
  fileName: string;
  relativePath: string;
  absolutePath: string;
  headingLevel: number | null;
  headingText: string | null;
  headingOrder: number | null;
  cite?: string | null;
  citeDate?: string | null;
  outlinePath: Array<{
    order: number;
    level: number;
    text: string;
  }>;
  snippet?: string | null;
  highlights: HighlightSpan[];
  score: number;
};

export type SearchWarning = {
  code: string;
  message: string;
};

export type SearchDiagnostics = {
  plannerMode:
    | "exact_id_like"
    | "short_keyword"
    | "phrase_like"
    | "path_like"
    | "name_like"
    | "long_mixed";
  latencyMs: {
    total: number;
    lexical: number;
    semantic?: number;
    rerank: number;
    payload: number;
  };
  candidateCounts: {
    exact: number;
    bm25f: number;
    prefixRescue: number;
    chargramRescue: number;
    semantic?: number;
    reranked: number;
  };
  semanticStatus?: "ready" | "stale" | "unavailable";
};

export type SearchResponse = {
  results: SearchHit[];
  totalApprox?: number;
  diagnostics?: SearchDiagnostics;
  warnings: SearchWarning[];
};

export type SearchInlineSpan = {
  start: number;
  end: number;
  kind: "highlight" | "underline" | "bold" | string;
  color?: "yellow" | "green" | "cyan" | "magenta" | "blue" | "gray" | string | null;
};

export type SearchCardParagraph = {
  paragraphIndex: number;
  text: string;
  spans: SearchInlineSpan[];
};

export type SearchOutlineEntry = {
  order: number;
  level: number;
  text: string;
};

export type SearchCardPayload = {
  resultId: number;
  entityType: "card";
  rootPath: string;
  fileId: number;
  fileName: string;
  relativePath: string;
  absolutePath: string;
  cardId: string;
  tag: string;
  tagSub: string;
  cite: string;
  citeDate?: string | null;
  headingOrder: number;
  headingLevel: number;
  headingTrail: SearchOutlineEntry[];
  body: SearchCardParagraph[];
};

export type SearchDocPayload = {
  resultId: number;
  entityType: "doc";
  rootPath: string;
  fileId: number;
  fileName: string;
  relativePath: string;
  absolutePath: string;
  headings: SearchOutlineEntry[];
};

export type HydratedSearchResult = SearchCardPayload | SearchDocPayload;

export type SearchHydrateResponse = {
  results: HydratedSearchResult[];
};

export type IndexStats = {
  scanned: number;
  updated: number;
  skipped: number;
  removed: number;
  headingsExtracted: number;
  elapsedMs: number;
};

export type IndexProgress = {
  rootPath: string;
  phase: "discovering" | "indexing" | "cleaning" | "committing" | "lexical" | "search" | "complete";
  discovered: number;
  changed: number;
  processed: number;
  updated: number;
  skipped: number;
  removed: number;
  elapsedMs: number;
  phaseElapsedMs: number;
  scanRatePerSec: number;
  processRatePerSec: number;
  etaMs: number | null;
  logPath: string | null;
  currentFile: string | null;
};

export type TreeRow = {
  key: string;
  kind: "folder" | "file" | "heading" | "f8" | "author" | "loading";
  depth: number;
  label: string;
  subLabel?: string;
  headingLevel?: number;
  headingOrder?: number;
  folderPath?: string;
  fileId?: number;
  copyText?: string;
  sourcePath?: string;
  richHtml?: string;
  paragraphXml?: string[];
  searchResult?: SearchHit;
  hasChildren?: boolean;
};

export type CaptureInsertResult = {
  capturePath: string;
  marker: string;
  targetRelativePath: string;
};

export type CaptureTarget = {
  relativePath: string;
  absolutePath: string;
  exists: boolean;
  entryCount: number;
};

export type CaptureTargetPreview = {
  relativePath: string;
  absolutePath: string;
  exists: boolean;
  headingCount: number;
  headings: FileHeading[];
};

export type SidePreview = {
  title: string;
  subTitle?: string;
  text: string;
  richHtml?: string;
  headingLevel?: number | null;
  kind?: "heading" | "f8" | "author";
};
