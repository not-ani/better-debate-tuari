import { For, Show, createEffect } from "solid-js";
import type { TexSessionRouteTarget } from "../lib/types";

type SpeechSendModalProps = {
  open: boolean;
  busy: boolean;
  sourceLabel: string;
  targets: TexSessionRouteTarget[];
  selectedTargetSessionId: string | null;
  rootSelected: boolean;
  selectedTargetBlockIndex: number | null;
  belowReason: string | null;
  underReason: string | null;
  onClose: () => void;
  onSelectTargetSession: (sessionId: string) => void;
  onSelectRoot: () => void;
  onSelectTargetBlock: (blockIndex: number) => void;
  onConfirm: (insertMode: "below" | "under") => void | Promise<void>;
};

export default function SpeechSendModal(props: SpeechSendModalProps) {
  let panelRef!: HTMLDivElement;
  let targetListRef!: HTMLDivElement;

  const selectedTarget = () =>
    props.targets.find((target) => target.sessionId === props.selectedTargetSessionId) ?? null;

  createEffect(() => {
    if (!props.open) {
      return;
    }

    queueMicrotask(() => {
      targetListRef?.focus();
      panelRef?.scrollTo({ top: 0 });
    });
  });

  const canConfirmBelow = () => !props.busy && !props.belowReason && (props.rootSelected || props.selectedTargetBlockIndex !== null);
  const canConfirmUnder = () =>
    !props.busy &&
    !props.underReason &&
    !props.rootSelected &&
    props.selectedTargetBlockIndex !== null;

  return (
    <Show when={props.open}>
      <div class="speech-send-backdrop" onMouseDown={props.onClose}>
        <section
          aria-label="Send to speech"
          class="speech-send-modal"
          onMouseDown={(event) => event.stopPropagation()}
          ref={panelRef}
          role="dialog"
        >
          <header class="speech-send-header">
            <div>
              <p class="speech-send-kicker">Send to speech</p>
              <h2 class="speech-send-title">{props.sourceLabel}</h2>
            </div>
            <button class="speech-send-close" onClick={props.onClose} type="button">
              Close
            </button>
          </header>

          <div class="speech-send-body">
            <div class="speech-send-pane">
              <div class="speech-send-pane-header">
                <span>Open Tex docs</span>
              </div>
              <div class="speech-send-target-list" ref={targetListRef} tabIndex={-1}>
                <For each={props.targets}>
                  {(target) => (
                    <button
                      class="speech-send-target"
                      classList={{ selected: target.sessionId === props.selectedTargetSessionId }}
                      onClick={() => props.onSelectTargetSession(target.sessionId)}
                      type="button"
                    >
                      <span class="speech-send-target-name">{target.fileName}</span>
                      <span class="speech-send-target-path">{target.filePath}</span>
                      <span class="speech-send-target-meta">
                        {target.ownerWindowLabel ? `Owner: ${target.ownerWindowLabel}` : "No writer"}
                      </span>
                    </button>
                  )}
                </For>
              </div>
            </div>

            <div class="speech-send-pane">
              <div class="speech-send-pane-header">
                <span>Destination outline</span>
              </div>

              <Show
                when={selectedTarget()}
                fallback={<p class="speech-send-empty">Choose a target document to pick a destination.</p>}
              >
                {(target) => (
                  <div class="speech-send-outline">
                    <button
                      class="speech-send-outline-row"
                      classList={{ selected: props.rootSelected }}
                      onClick={props.onSelectRoot}
                      type="button"
                    >
                      <span class="speech-send-level-badge root">Root</span>
                      <span class="speech-send-outline-text">Document Root</span>
                    </button>

                    <Show
                      when={target().headings.length > 0}
                      fallback={<p class="speech-send-empty">This document has no H1-H4 headings yet.</p>}
                    >
                      <For each={target().headings}>
                        {(heading) => (
                          <button
                            class="speech-send-outline-row"
                            classList={{
                              selected:
                                !props.rootSelected &&
                                heading.blockIndex === props.selectedTargetBlockIndex,
                            }}
                            onClick={() => props.onSelectTargetBlock(heading.blockIndex)}
                            style={{ "padding-left": `${0.85 + (heading.level - 1) * 0.75}rem` }}
                            type="button"
                          >
                            <span class="speech-send-level-badge">H{heading.level}</span>
                            <span class="speech-send-outline-text">{heading.text}</span>
                          </button>
                        )}
                      </For>
                    </Show>
                  </div>
                )}
              </Show>
            </div>
          </div>

          <footer class="speech-send-footer">
            <div class="speech-send-mode-group">
              <button
                class="speech-send-action"
                disabled={!canConfirmBelow()}
                onClick={() => void props.onConfirm("below")}
                type="button"
              >
                Below
              </button>
              <button
                class="speech-send-action"
                disabled={!canConfirmUnder()}
                onClick={() => void props.onConfirm("under")}
                type="button"
              >
                Under
              </button>
            </div>

            <div class="speech-send-reasons">
              <Show when={props.belowReason}>
                <p>{props.belowReason}</p>
              </Show>
              <Show when={!props.belowReason && props.underReason}>
                <p>{props.underReason}</p>
              </Show>
            </div>
          </footer>
        </section>
      </div>
    </Show>
  );
}
