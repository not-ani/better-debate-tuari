import { expect, test } from "bun:test";
import { createTexEditorState } from "./state";

test("default paste does not force plain-text insertion", () => {
  const state = createTexEditorState(
    {
      filePath: "/tmp/test.tex",
      fileName: "test.tex",
      paragraphCount: 1,
      blocks: [
        {
          id: "block-1",
          kind: "paragraph",
          text: "hello",
          runs: [],
          level: null,
          styleId: "Normal",
          styleName: "Normal",
          isF8Cite: false,
        },
      ],
    },
    () => {},
    () => false,
    () => false,
  );

  expect(state.plugins.some((plugin) => typeof plugin.props.handlePaste === "function")).toBeFalse();
});
