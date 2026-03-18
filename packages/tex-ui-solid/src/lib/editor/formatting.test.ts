import { expect, test } from "bun:test";
import { EditorState, TextSelection } from "prosemirror-state";
import type { EditorView } from "prosemirror-view";
import { texEditorSchema } from "../editor-schema";
import { clearToNormal, setHeadingLevel } from "./formatting";

const createMockView = (state: EditorState) => {
  const view = {
    state,
    dispatch(transaction) {
      view.state = view.state.apply(transaction);
    },
    focus() {},
  };

  return view as unknown as EditorView;
};

const createParagraphState = (text: string, position: number) => {
  const paragraph = texEditorSchema.nodes.paragraph.create(null, [texEditorSchema.text(text)]);
  const doc = texEditorSchema.nodes.doc.create(null, [paragraph]);

  return EditorState.create({
    schema: texEditorSchema,
    doc,
    selection: TextSelection.create(doc, position),
  });
};

test.each([1, 2, 3])(
  "setHeadingLevel preserves the caret position when converting a paragraph to heading level %i",
  (level) => {
    const state = createParagraphState("hat", 4);
    const view = createMockView(state);

    setHeadingLevel(view, level);

    expect(view.state.doc.firstChild?.type.name).toBe("heading");
    expect(view.state.doc.firstChild?.attrs.level).toBe(level);
    expect(view.state.selection.from).toBe(4);
    expect(view.state.selection.to).toBe(4);
  },
);

test("clearToNormal preserves the caret position when converting a heading", () => {
  const heading = texEditorSchema.nodes.heading.create(
    { level: 2, styleId: "Heading2", styleName: "Heading 2", isF8Cite: false },
    [texEditorSchema.text("video")],
  );
  const doc = texEditorSchema.nodes.doc.create(null, [heading]);
  const state = EditorState.create({
    schema: texEditorSchema,
    doc,
    selection: TextSelection.create(doc, 1),
  });
  const view = createMockView(state);

  clearToNormal(view);

  expect(view.state.doc.firstChild?.type.name).toBe("paragraph");
  expect(view.state.selection.from).toBe(1);
  expect(view.state.selection.to).toBe(1);
});
