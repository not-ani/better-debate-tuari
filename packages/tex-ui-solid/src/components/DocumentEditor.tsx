import { createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { EditorState, Selection, TextSelection } from "prosemirror-state";
import { EditorView } from "prosemirror-view";
import EditorRibbon, { type RibbonAction } from "./EditorRibbon";
import { condenseAllOrSelection, pastePlainText } from "../lib/editor/condense";
import {
  clearToNormal,
  isCiteActive,
  isHeadingActive,
  isMarkActive,
  setHeadingLevel,
  toggleCiteStyle,
  toggleHighlight,
  toggleStrong,
  toggleUnderlineStyle,
} from "../lib/editor/formatting";
import { createTexEditorState, scrollToBlock } from "../lib/editor/state";
import {
  getStoredMarksWithoutStickyHighlight,
  STICKY_HIGHLIGHT_CLEAR_META,
} from "../lib/editor/sticky-highlight";
import type { TexSessionSnapshot } from "../lib/types";
import { pmDocToTexBlocks, texBlocksToPmDoc, texEditorSchema } from "../lib/editor-schema";

const EDITOR_ZOOM_STORAGE_KEY = "tex-editor-zoom";
const DEFAULT_EDITOR_ZOOM = 1;
const MIN_EDITOR_ZOOM = 0.5;
const MAX_EDITOR_ZOOM = 2;
const KEYBOARD_ZOOM_STEP = 0.1;
const ZOOM_EPSILON = 0.001;

type ZoomFocusPoint = {
  clientX: number;
  clientY: number;
};

type WebKitGestureEvent = Event & {
  scale: number;
  clientX: number;
  clientY: number;
};

const clampEditorZoom = (value: number) =>
  Math.min(MAX_EDITOR_ZOOM, Math.max(MIN_EDITOR_ZOOM, Number.isFinite(value) ? value : DEFAULT_EDITOR_ZOOM));

const roundEditorZoom = (value: number) => Math.round(clampEditorZoom(value) * 100) / 100;

const getInitialEditorZoom = () => {
  if (typeof window === "undefined") {
    return DEFAULT_EDITOR_ZOOM;
  }

  const storedValue = Number.parseFloat(window.localStorage.getItem(EDITOR_ZOOM_STORAGE_KEY) ?? "");
  return clampEditorZoom(storedValue);
};

const persistEditorZoom = (zoom: number) => {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(EDITOR_ZOOM_STORAGE_KEY, String(roundEditorZoom(zoom)));
};

const isEditableTarget = (target: EventTarget | null) => {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  return Boolean(target.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']"));
};

const getZoomCommand = (event: KeyboardEvent): "in" | "out" | "reset" | null => {
  if (!(event.metaKey || event.ctrlKey) || event.altKey) {
    return null;
  }

  if (event.code === "NumpadAdd") {
    return "in";
  }
  if (event.code === "NumpadSubtract") {
    return "out";
  }

  switch (event.key) {
    case "+":
    case "=":
      return "in";
    case "-":
    case "_":
      return "out";
    case "0":
      return "reset";
    default:
      return null;
  }
};

const isWebKitGestureEvent = (event: Event): event is WebKitGestureEvent =>
  "scale" in event && "clientX" in event && "clientY" in event;

const runsEqual = (
  left: TexSessionSnapshot["blocks"][number]["runs"][number],
  right: TexSessionSnapshot["blocks"][number]["runs"][number],
) =>
  left.text === right.text &&
  left.bold === right.bold &&
  left.italic === right.italic &&
  left.underline === right.underline &&
  left.smallCaps === right.smallCaps &&
  left.highlightColor === right.highlightColor &&
  left.styleId === right.styleId &&
  left.styleName === right.styleName &&
  left.isF8Cite === right.isF8Cite;

const blocksEqualIgnoringIds = (
  left: TexSessionSnapshot["blocks"],
  right: TexSessionSnapshot["blocks"],
) => {
  if (left.length !== right.length) {
    return false;
  }

  return left.every((leftBlock, index) => {
    const rightBlock = right[index];
    if (!rightBlock) {
      return false;
    }

    if (
      leftBlock.kind !== rightBlock.kind ||
      leftBlock.text !== rightBlock.text ||
      leftBlock.level !== rightBlock.level ||
      leftBlock.styleId !== rightBlock.styleId ||
      leftBlock.styleName !== rightBlock.styleName ||
      leftBlock.isF8Cite !== rightBlock.isF8Cite ||
      leftBlock.runs.length !== rightBlock.runs.length
    ) {
      return false;
    }

    return leftBlock.runs.every((leftRun, runIndex) => {
      const rightRun = rightBlock.runs[runIndex];
      return !!rightRun && runsEqual(leftRun, rightRun);
    });
  });
};

const clampDocPosition = (doc: EditorState["doc"], position: number) =>
  Math.max(0, Math.min(position, doc.content.size));

const selectionForNextDoc = (doc: EditorState["doc"], previousSelection: Selection) => {
  const anchor = clampDocPosition(doc, previousSelection.anchor);
  const head = clampDocPosition(doc, previousSelection.head);

  if (previousSelection.empty) {
    return Selection.near(doc.resolve(anchor), 1);
  }

  try {
    return TextSelection.between(doc.resolve(anchor), doc.resolve(head), 1);
  } catch {
    return Selection.near(doc.resolve(anchor), 1);
  }
};

type DocumentEditorProps = {
  document: TexSessionSnapshot;
  invisibilityMode: boolean;
  stickyHighlightMode: boolean;
  saving: boolean;
  scrollToBlockIndex: number | null;
  ribbonOpen: boolean;
  speechSendOpen: boolean;
  onDocumentChange: (next: TexSessionSnapshot) => void;
  onActiveBlockIndexChange: (blockIndex: number | null) => void;
  onNewFile: () => void | Promise<void>;
  onOpenSpeechSend: (forceTargetPick: boolean) => void;
  onSave: () => void | Promise<void>;
  onToggleInvisibilityMode: () => void;
  onToggleStickyHighlightMode: () => void;
  onToggleRibbon: () => void;
};

export default function DocumentEditor(props: DocumentEditorProps) {
  let host!: HTMLDivElement;
  let editorPageRef!: HTMLDivElement;
  let view: EditorView | null = null;
  const [revision, setRevision] = createSignal(0);
  const [loadedSessionKey, setLoadedSessionKey] = createSignal("");
  const [draggingMargin, setDraggingMargin] = createSignal(false);
  const [zoom, setZoom] = createSignal(getInitialEditorZoom());

  const activeView = () => {
    revision();
    return view;
  };

  const activeBlockIndexFromSelection = (state: EditorState) => state.selection.$from.index(0);

  const publishActiveBlockIndex = (state: EditorState) => {
    props.onActiveBlockIndexChange(activeBlockIndexFromSelection(state));
  };

  const applyZoom = (nextZoom: number, focusPoint?: ZoomFocusPoint) => {
    const page = editorPageRef;
    const currentZoom = zoom();
    const clampedZoom = roundEditorZoom(nextZoom);

    if (!page || Math.abs(clampedZoom - currentZoom) < ZOOM_EPSILON) {
      setZoom(clampedZoom);
      return;
    }

    const rect = page.getBoundingClientRect();
    const anchor = focusPoint ?? {
      clientX: rect.left + rect.width / 2,
      clientY: rect.top + rect.height / 2,
    };
    const relativeX = anchor.clientX - rect.left;
    const relativeY = anchor.clientY - rect.top;
    const contentX = (page.scrollLeft + relativeX) / currentZoom;
    const contentY = (page.scrollTop + relativeY) / currentZoom;

    setZoom(clampedZoom);

    queueMicrotask(() => {
      page.scrollLeft = contentX * clampedZoom - relativeX;
      page.scrollTop = contentY * clampedZoom - relativeY;
    });
  };

  const stepZoom = (direction: 1 | -1, focusPoint?: ZoomFocusPoint) => {
    applyZoom(zoom() + KEYBOARD_ZOOM_STEP * direction, focusPoint);
  };

  const refreshViewFromDocument = (
    nextDocument: TexSessionSnapshot,
    options?: { preserveSelection: boolean },
  ) => {
    if (!view) {
      return;
    }

    const previousSelection = view.state.selection;
    const nextDoc = texBlocksToPmDoc(nextDocument.blocks);
    let nextState = EditorState.create({
      schema: texEditorSchema,
      doc: nextDoc,
      plugins: view.state.plugins,
    });

    if (options?.preserveSelection) {
      nextState = nextState.apply(
        nextState.tr.setSelection(selectionForNextDoc(nextState.doc, previousSelection)),
      );
    }

    view.updateState(nextState);
    setLoadedSessionKey(`${nextDocument.sessionId}:${nextDocument.version}`);
    setRevision((value) => value + 1);
    publishActiveBlockIndex(nextState);
  };

  const startMarginDrag = (side: "left" | "right", event: MouseEvent) => {
    event.preventDefault();
    setDraggingMargin(true);

    const page = editorPageRef;
    if (!page) {
      return;
    }

    const onMove = (moveEvent: MouseEvent) => {
      const rect = page.getBoundingClientRect();
      const margin =
        side === "left"
          ? Math.max(16, moveEvent.clientX - rect.left)
          : Math.max(16, rect.right - moveEvent.clientX);
      const clamped = Math.min(margin, rect.width * 0.35);
      page.style.setProperty("--editor-margin", `${clamped}px`);
    };

    const onUp = () => {
      setDraggingMargin(false);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };

    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  onMount(() => {
    const spellcheckLanguage = navigator.language || "en-US";
    let gestureStartZoom = zoom();
    let gestureFocusPoint: ZoomFocusPoint | undefined;

    const runFunctionKey = (key: string) => {
      if (!view) {
        return false;
      }

      switch (key) {
        case "F2":
          void pastePlainText(view);
          return true;
        case "F3":
          condenseAllOrSelection(view);
          return true;
        case "F4":
          setHeadingLevel(view, 1);
          return true;
        case "F5":
          setHeadingLevel(view, 2);
          return true;
        case "F6":
          setHeadingLevel(view, 3);
          return true;
        case "F7":
          setHeadingLevel(view, 4);
          return true;
        case "F8":
          toggleCiteStyle(view);
          return true;
        case "F9":
          toggleUnderlineStyle(view);
          return true;
        case "F10":
          toggleStrong(view);
          return true;
        case "F11":
          toggleHighlight(view);
          return true;
        case "F12":
          clearToNormal(view);
          return true;
        default:
          return false;
      }
    };

    view = new EditorView(host, {
      state: createTexEditorState(
        props.document,
        props.onSave,
        runFunctionKey,
        () => props.stickyHighlightMode,
      ),
      attributes: {
        spellcheck: "true",
        lang: spellcheckLanguage,
        autocorrect: "on",
        autocapitalize: "sentences",
      },
      dispatchTransaction(transaction) {
        if (!view) {
          return;
        }

        const nextState = view.state.apply(transaction);
        view.updateState(nextState);
        setRevision((value) => value + 1);
        publishActiveBlockIndex(nextState);

        if (!transaction.docChanged) {
          return;
        }

        props.onDocumentChange({
          ...props.document,
          paragraphCount: nextState.doc.childCount,
          dirty: true,
          blocks: pmDocToTexBlocks(nextState.doc),
        });
      },
    });
    view.dom.spellcheck = true;
    view.dom.setAttribute("lang", spellcheckLanguage);
    view.dom.setAttribute("autocorrect", "on");
    view.dom.setAttribute("autocapitalize", "sentences");
    setLoadedSessionKey(`${props.document.sessionId}:${props.document.version}`);
    publishActiveBlockIndex(view.state);

    const onWindowKeyDown = (event: KeyboardEvent) => {
      if (
        !props.speechSendOpen &&
        !event.metaKey &&
        !event.ctrlKey &&
        !event.altKey &&
        !(isEditableTarget(event.target) && event.target instanceof Node && !host.contains(event.target)) &&
        event.code === "Backquote"
      ) {
        event.preventDefault();
        props.onOpenSpeechSend(event.shiftKey);
        return;
      }

      const zoomCommand = getZoomCommand(event);
      if (zoomCommand) {
        if (isEditableTarget(event.target) && event.target instanceof Node && !host.contains(event.target)) {
          return;
        }

        event.preventDefault();
        if (zoomCommand === "in") {
          stepZoom(1);
        } else if (zoomCommand === "out") {
          stepZoom(-1);
        } else {
          applyZoom(DEFAULT_EDITOR_ZOOM);
        }
        return;
      }

      if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey) {
        return;
      }

      if (event.target instanceof Node && host.contains(event.target)) {
        return;
      }

      if (!/^F([2-9]|1[0-2])$/.test(event.key)) {
        return;
      }

      if (runFunctionKey(event.key)) {
        event.preventDefault();
      }
    };

    const onZoomWheel = (event: WheelEvent) => {
      if (!event.ctrlKey) {
        return;
      }

      if (!(event.target instanceof Node) || !editorPageRef.contains(event.target)) {
        return;
      }

      event.preventDefault();

      const multiplier = Math.exp(-event.deltaY * 0.0025);
      applyZoom(zoom() * multiplier, { clientX: event.clientX, clientY: event.clientY });
    };

    const onGestureStart = (event: Event) => {
      if (!isWebKitGestureEvent(event)) {
        return;
      }

      event.preventDefault();
      gestureStartZoom = zoom();
      gestureFocusPoint = { clientX: event.clientX, clientY: event.clientY };
    };

    const onGestureChange = (event: Event) => {
      if (!isWebKitGestureEvent(event)) {
        return;
      }

      event.preventDefault();
      applyZoom(gestureStartZoom * event.scale, gestureFocusPoint);
    };

    window.addEventListener("keydown", onWindowKeyDown, { capture: true });
    editorPageRef.addEventListener("wheel", onZoomWheel, { passive: false });
    editorPageRef.addEventListener("gesturestart", onGestureStart, { passive: false });
    editorPageRef.addEventListener("gesturechange", onGestureChange, { passive: false });
    onCleanup(() => {
      window.removeEventListener("keydown", onWindowKeyDown, { capture: true });
      editorPageRef.removeEventListener("wheel", onZoomWheel);
      editorPageRef.removeEventListener("gesturestart", onGestureStart);
      editorPageRef.removeEventListener("gesturechange", onGestureChange);
    });
  });

  onCleanup(() => {
    view?.destroy();
    view = null;
  });

  createEffect(() => {
    persistEditorZoom(zoom());
  });

  createEffect(() => {
    if (!view) {
      return;
    }

    const nextSessionKey = `${props.document.sessionId}:${props.document.version}`;
    if (nextSessionKey !== loadedSessionKey()) {
      const currentBlocks = pmDocToTexBlocks(view.state.doc);
      if (blocksEqualIgnoringIds(currentBlocks, props.document.blocks)) {
        setLoadedSessionKey(nextSessionKey);
        return;
      }

      const loadedSessionId = loadedSessionKey().split(":")[0] ?? "";
      refreshViewFromDocument(props.document, {
        preserveSelection: loadedSessionId === props.document.sessionId,
      });
    }
  });

  createEffect((wasEnabled?: boolean) => {
    const enabled = props.stickyHighlightMode;
    const current = view;
    if (!current) {
      return enabled;
    }

    if (!enabled && wasEnabled) {
      const clearedStoredMarks = getStoredMarksWithoutStickyHighlight(current.state);
      if (clearedStoredMarks) {
        current.dispatch(
          current.state.tr
            .setStoredMarks(clearedStoredMarks)
            .setMeta(STICKY_HIGHLIGHT_CLEAR_META, true),
        );
      }
    }

    return enabled;
  });

  createEffect(() => {
    const blockIndex = props.scrollToBlockIndex;
    if (blockIndex == null || !view) {
      return;
    }
    scrollToBlock(view, blockIndex);
  });

  const runWithView = (callback: (current: EditorView) => void) => {
    const current = activeView();
    if (!current) {
      return;
    }
    callback(current);
    current.focus();
  };

  const actions = createMemo<RibbonAction[]>(() => {
    const current = activeView();

    return [
      {
        key: "F2",
        label: "Paste",
        group: "Clipboard",
        active: false,
        run: () => {
          if (current) {
            void pastePlainText(current);
          }
        },
      },
      {
        key: "F3",
        label: "Condense",
        group: "Clipboard",
        active: false,
        run: () => {
          if (current) {
            condenseAllOrSelection(current);
          }
        },
      },
      {
        key: "F4",
        label: "Pocket",
        group: "Structure",
        active: current ? isHeadingActive(current, 1) : false,
        run: () => runWithView((editor) => setHeadingLevel(editor, 1)),
      },
      {
        key: "F5",
        label: "Hat",
        group: "Structure",
        active: current ? isHeadingActive(current, 2) : false,
        run: () => runWithView((editor) => setHeadingLevel(editor, 2)),
      },
      {
        key: "F6",
        label: "Block",
        group: "Structure",
        active: current ? isHeadingActive(current, 3) : false,
        run: () => runWithView((editor) => setHeadingLevel(editor, 3)),
      },
      {
        key: "F7",
        label: "Tag",
        group: "Structure",
        active: current ? isHeadingActive(current, 4) : false,
        run: () => runWithView((editor) => setHeadingLevel(editor, 4)),
      },
      {
        key: "F8",
        label: "Cite",
        group: "Structure",
        active: current ? isCiteActive(current) : false,
        run: () => runWithView(toggleCiteStyle),
      },
      {
        key: "F9",
        label: "Underline",
        group: "Format",
        active: current ? isMarkActive(current, "underline") : false,
        run: () => runWithView(toggleUnderlineStyle),
      },
      {
        key: "F10",
        label: "Emphasis",
        group: "Format",
        active: current ? isMarkActive(current, "strong") : false,
        run: () => runWithView(toggleStrong),
      },
      {
        key: "F11",
        label: "Highlight",
        group: "Format",
        active: current ? isMarkActive(current, "highlight") : false,
        run: () => runWithView(toggleHighlight),
      },
      {
        key: "F12",
        label: "Clear",
        group: "Format",
        active: false,
        run: () => runWithView(clearToNormal),
      },
      {
        key: "Cmd/Ctrl+Shift+I",
        label: "Invisible",
        group: "View",
        active: props.invisibilityMode,
        run: props.onToggleInvisibilityMode,
      },
    ];
  });

  return (
    <div class="editor-shell">
      <EditorRibbon
        actions={actions()}
        disabled={!activeView() || props.saving}
        invisibilityMode={props.invisibilityMode}
        stickyHighlightMode={props.stickyHighlightMode}
        onNewFile={props.onNewFile}
        onToggleInvisibilityMode={props.onToggleInvisibilityMode}
        onToggleStickyHighlightMode={props.onToggleStickyHighlightMode}
        onSave={props.onSave}
        saving={props.saving}
        ribbonOpen={props.ribbonOpen}
        onToggleRibbon={props.onToggleRibbon}
      />

      <div
        class="editor-page"
        classList={{ "invisibility-mode": props.invisibilityMode }}
        ref={editorPageRef}
        style={{ "--editor-zoom": String(zoom()) }}
      >
        <div
          class="editor-margin-handle editor-margin-handle--left"
          classList={{ dragging: draggingMargin() }}
          onMouseDown={(event) => startMarginDrag("left", event)}
        />
        <div
          class="editor-margin-handle editor-margin-handle--right"
          classList={{ dragging: draggingMargin() }}
          onMouseDown={(event) => startMarginDrag("right", event)}
        />
        <div class="editor-surface" ref={host} />
      </div>
    </div>
  );
}
