import { expect, test } from "bun:test";
import { texBlocksToPmDoc, texEditorSchema } from "./editor-schema";

test("paragraph clipboard attrs preserve debate-specific metadata", () => {
  const paragraphSpec = texEditorSchema.spec.nodes.get("paragraph");
  const parseRule = paragraphSpec?.parseDOM?.[0];

  expect(parseRule && typeof parseRule.getAttrs === "function").toBeTrue();

  const attrs = parseRule!.getAttrs!({
    dataset: {
      styleId: "Cite",
      styleName: "Cite",
      f8Cite: "true",
    },
  } as unknown as HTMLElement);

  expect(attrs).toEqual({
    styleId: "Cite",
    styleName: "Cite",
    isF8Cite: true,
  });
});

test("heading DOM output includes full block metadata for clipboard round-trips", () => {
  const doc = texBlocksToPmDoc([
    {
      id: "block-1",
      kind: "heading",
      text: "Tag",
      runs: [],
      level: 4,
      styleId: "Heading4",
      styleName: "Tag",
      isF8Cite: true,
    },
  ]);

  const heading = doc.firstChild;
  const output = texEditorSchema.nodes.heading.spec.toDOM?.(heading!);

  expect(output).toEqual([
    "h4",
    {
      "data-style-id": "Heading4",
      "data-style-name": "Tag",
      "data-f8-cite": "true",
    },
    0,
  ]);
});
