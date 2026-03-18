import type { IndexProgress } from "./types";

export const formatElapsed = (elapsedMs: number) => {
  const totalSeconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return minutes > 0 ? `${minutes}m ${seconds}s` : `${seconds}s`;
};

export const formatRate = (value: number) => {
  if (!Number.isFinite(value) || value <= 0) return "0/s";
  return `${value >= 10 ? value.toFixed(0) : value.toFixed(1)}/s`;
};

export const getEstimatedTimeRemaining = (progress: IndexProgress | null) => {
  if (!progress || progress.phase !== "indexing" || progress.etaMs == null || progress.etaMs <= 0) {
    return null;
  }
  return formatElapsed(progress.etaMs);
};

export const getIndexProgressPercent = (progress: IndexProgress | null) => {
  if (!progress || progress.changed <= 0) return 0;
  const rawPercent = (progress.processed / progress.changed) * 100;
  if (progress.phase !== "complete" && progress.processed < progress.changed) {
    return Math.min(99, Math.floor(rawPercent));
  }
  return Math.min(100, Math.round(rawPercent));
};

export const getIndexProgressTitle = (progress: IndexProgress | null, options?: { includeDocxLabel?: boolean }) => {
  if (!progress) return "";

  if (progress.phase === "discovering") {
    const noun = options?.includeDocxLabel ? " .docx files" : " files";
    return `Scanning ${progress.discovered.toLocaleString()}${noun}`;
  }

  if (progress.phase === "indexing") {
    const remaining = Math.max(0, progress.changed - progress.processed);
    return `Indexing ${progress.processed.toLocaleString()} / ${progress.changed.toLocaleString()} files${
      remaining > 0 ? ` (${remaining.toLocaleString()} left)` : ""
    }`;
  }

  if (progress.phase === "cleaning") return "Cleaning stale entries";
  if (progress.phase === "committing") return "Committing database changes";
  if (progress.phase === "lexical") {
    return options?.includeDocxLabel ? "Building lexical shards (final step)" : "Building lexical index";
  }
  if (progress.phase === "search") return "Updating search index";
  return options?.includeDocxLabel ? "Finalizing index" : "Finalizing";
};

export const getIndexProgressDetail = (progress: IndexProgress | null) => {
  if (!progress) return "";
  if (progress.phase === "lexical") {
    return `Final pass after document parsing. Elapsed ${formatElapsed(progress.elapsedMs)}.`;
  }
  if (progress.phase === "search") {
    return `Refreshing search artifacts - elapsed ${formatElapsed(progress.phaseElapsedMs)}`;
  }
  if (progress.phase === "committing") {
    return `Writing the index transaction to disk - elapsed ${formatElapsed(progress.phaseElapsedMs)}`;
  }
  if (progress.phase === "cleaning" && progress.currentFile) {
    return `Removing ${progress.currentFile} - ${formatElapsed(progress.elapsedMs)}`;
  }
  if (progress.currentFile) {
    const eta = getEstimatedTimeRemaining(progress);
    return eta
      ? `${progress.currentFile} - elapsed ${formatElapsed(progress.elapsedMs)} - ETA ${eta}`
      : `${progress.currentFile} - elapsed ${formatElapsed(progress.elapsedMs)}`;
  }
  return `Elapsed ${formatElapsed(progress.elapsedMs)}`;
};

export const getIndexProgressStageLabel = (progress: IndexProgress | null) => {
  const phase = progress?.phase;
  if (!phase) return "";
  if (phase === "discovering") return "Stage 1/6";
  if (phase === "indexing") return "Stage 2/6";
  if (phase === "cleaning") return "Stage 3/6";
  if (phase === "committing") return "Stage 4/6";
  if (phase === "lexical") return "Stage 5/6";
  if (phase === "search") return "Stage 6/6";
  return "Finishing";
};

export const getIndexProgressForRoot = (progress: IndexProgress | null, rootPath: string) => {
  if (!progress || progress.rootPath !== rootPath) return null;
  return progress;
};

export const getProgressLabelForRoot = (progress: IndexProgress | null, rootPath: string) => {
  const matched = getIndexProgressForRoot(progress, rootPath);
  if (!matched) return "Idle";

  if (matched.phase === "discovering") return `Scanning ${matched.discovered.toLocaleString()} files`;
  if (matched.phase === "indexing") {
    return `Indexing ${matched.processed.toLocaleString()} / ${matched.changed.toLocaleString()}`;
  }
  if (matched.phase === "cleaning") return `Cleaning stale entries (${matched.removed.toLocaleString()} removed)`;
  if (matched.phase === "committing") return "Committing database changes";
  if (matched.phase === "lexical") return "Building lexical shards";
  if (matched.phase === "search") return "Updating search index";
  return "Finalizing";
};

export const getProgressPercentForRoot = (progress: IndexProgress | null, rootPath: string) => {
  const matched = getIndexProgressForRoot(progress, rootPath);
  if (!matched || matched.phase !== "indexing") return null;
  return getIndexProgressPercent(matched);
};
