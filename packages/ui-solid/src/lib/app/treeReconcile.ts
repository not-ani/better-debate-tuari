import type { SearchHit, TreeRow } from "../types";

const sameSearchResult = (left: SearchHit | undefined, right: SearchHit | undefined) => {
  if (left === right) return true;
  if (!left || !right) return false;

  return (
    left.source === right.source &&
    left.kind === right.kind &&
    left.fileId === right.fileId &&
    left.fileName === right.fileName &&
    left.relativePath === right.relativePath &&
    left.absolutePath === right.absolutePath &&
    left.headingLevel === right.headingLevel &&
    left.headingText === right.headingText &&
    left.headingOrder === right.headingOrder &&
    left.score === right.score
  );
};

const sameParagraphXml = (left: string[] | undefined, right: string[] | undefined) => {
  if (left === right) return true;
  if (!left || !right || left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
};

const sameTreeRow = (left: TreeRow, right: TreeRow) => {
  if (left === right) return true;
  return (
    left.key === right.key &&
    left.kind === right.kind &&
    left.depth === right.depth &&
    left.label === right.label &&
    left.subLabel === right.subLabel &&
    left.headingLevel === right.headingLevel &&
    left.headingOrder === right.headingOrder &&
    left.folderPath === right.folderPath &&
    left.fileId === right.fileId &&
    left.copyText === right.copyText &&
    left.sourcePath === right.sourcePath &&
    left.richHtml === right.richHtml &&
    left.hasChildren === right.hasChildren &&
    sameParagraphXml(left.paragraphXml, right.paragraphXml) &&
    sameSearchResult(left.searchResult, right.searchResult)
  );
};

export const reconcileTreeRows = (previousRows: TreeRow[], nextRows: TreeRow[]) => {
  if (previousRows.length === 0 || nextRows.length === 0) return nextRows;

  const previousByKey = new Map<string, TreeRow>();
  for (const previousRow of previousRows) {
    previousByKey.set(previousRow.key, previousRow);
  }

  let reusedCount = 0;
  const reconciled = nextRows.map((nextRow) => {
    const previousRow = previousByKey.get(nextRow.key);
    if (previousRow && sameTreeRow(previousRow, nextRow)) {
      reusedCount += 1;
      return previousRow;
    }
    return nextRow;
  });

  if (reusedCount === nextRows.length && previousRows.length === nextRows.length) {
    return previousRows;
  }

  return reconciled;
};
