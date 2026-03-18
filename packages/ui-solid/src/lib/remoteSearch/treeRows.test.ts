import { expect, test } from "bun:test";
import { buildDebatifyTagTreeRows } from "./treeRows";
import type { DebatifyTagHit } from "./types";

const hits: DebatifyTagHit[] = [
  {
    id: "one",
    tag: "Climate impact",
    citation: "Doe, 2024",
    richHtml: "<p>Evidence one</p>",
    plainText: "Evidence one",
    copyText: "Climate impact\n\nDoe, 2024\n\nEvidence one",
    paragraphXml: ["<w:p>one</w:p>"],
    sourcePath: "https://api.debatify.app/search?q=climate#result-1",
  },
  {
    id: "two",
    tag: "Economic cost",
    citation: "",
    richHtml: "<p>Evidence two</p>",
    plainText: "Evidence two",
    copyText: "Economic cost\n\nEvidence two",
    paragraphXml: ["<w:p>two</w:p>"],
    sourcePath: "https://api.debatify.app/search?q=climate#result-2",
  },
];

test("returns only the remote folder row when collapsed", () => {
  const rows = buildDebatifyTagTreeRows(hits, false);

  expect(rows).toHaveLength(1);
  expect(rows[0]).toMatchObject({
    kind: "folder",
    label: "Debatify API tags",
    subLabel: "2 matches",
    folderPath: "remote:debatify",
  });
});

test("returns heading rows for each hit when expanded", () => {
  const rows = buildDebatifyTagTreeRows(hits, true);

  expect(rows).toHaveLength(3);
  expect(rows[1]).toMatchObject({
    kind: "heading",
    label: "Climate impact",
    subLabel: "H4 - Doe, 2024",
    headingLevel: 4,
  });
  expect(rows[2]).toMatchObject({
    kind: "heading",
    label: "Economic cost",
    subLabel: "H4 - Debatify API tag",
    headingLevel: 4,
  });
});
