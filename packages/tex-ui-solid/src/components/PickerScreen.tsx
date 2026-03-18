import { For, Show } from "solid-js";
import type { RecentFile, TexRecoverableSession } from "../lib/types";
import type { ThemeMode } from "../lib/theme";

const formatOpenedAt = (openedAtMs: number) =>
  new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(openedAtMs);

type PickerScreenProps = {
  busy: boolean;
  recentFiles: RecentFile[];
  recoverableSessions: TexRecoverableSession[];
  settingsLabel: string;
  theme: ThemeMode;
  onDiscardRecovery: (sessionId: string) => void;
  onNewSpeech: () => void;
  onOpenDialog: () => void;
  onOpenSearch: () => void;
  onOpenRecent: (path: string) => void;
  onRecoverSession: (sessionId: string) => void;
  onOpenSettings: () => void;
  onToggleTheme: () => void;
};

export default function PickerScreen(props: PickerScreenProps) {
  return (
    <main class="picker-screen">
      <header class="picker-header">
        <button class="wordmark" disabled={props.busy} onClick={props.onOpenDialog} type="button">
          <span class="wordmark-mark">T</span>
          <span class="wordmark-name">Tex</span>
        </button>

        <div class="picker-actions">
          <button class="theme-toggle" onClick={props.onToggleTheme} type="button">
            {props.theme === "light" ? "Dark mode" : "Light mode"}
          </button>
          <button class="picker-open-link" onClick={props.onOpenSearch} type="button">
            Search
          </button>
          <button class="picker-open-link" onClick={props.onOpenSettings} type="button">
            {props.settingsLabel}
          </button>
          <button
            class="picker-open-link"
            disabled={props.busy}
            onClick={props.onNewSpeech}
            type="button"
          >
            New speech
          </button>
          <button
            class="picker-open-link"
            disabled={props.busy}
            onClick={props.onOpenDialog}
            type="button"
          >
            {props.busy ? "Opening..." : "Open file"}
          </button>
        </div>
      </header>

      <Show when={props.recoverableSessions.length > 0}>
        <section class="recovery-shell" aria-label="Recover unsaved sessions">
          <div class="recovery-header">
            <div>
              <p class="recovery-eyebrow">Recovery</p>
              <h2 class="recovery-title">Unsaved sessions were found.</h2>
            </div>
            <p class="recovery-copy">
              Recover the latest temp snapshot or discard it permanently.
            </p>
          </div>

          <div class="recovery-list">
            <For each={props.recoverableSessions}>
              {(session) => (
                <article class="recovery-item">
                  <div class="recovery-meta">
                    <strong>{session.fileName}</strong>
                    <span>{formatOpenedAt(session.updatedAtMs)}</span>
                    <span class="recovery-path">{session.filePath}</span>
                  </div>

                  <div class="recovery-actions">
                    <button
                      class="picker-open-link"
                      onClick={() => props.onRecoverSession(session.sessionId)}
                      type="button"
                    >
                      Recover
                    </button>
                    <button
                      class="picker-secondary-link"
                      onClick={() => props.onDiscardRecovery(session.sessionId)}
                      type="button"
                    >
                      Discard
                    </button>
                  </div>
                </article>
              )}
            </For>
          </div>
        </section>
      </Show>

      <section class="recent-table-shell" aria-label="Recent files">
        <table class="recent-table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Opened</th>
              <th>Location</th>
            </tr>
          </thead>
          <tbody>
            <Show
              when={props.recentFiles.length > 0}
              fallback={
                <tr>
                  <td class="recent-empty" colSpan={3}>
                    No recent files yet.
                  </td>
                </tr>
              }
            >
              <For each={props.recentFiles}>
                {(file) => (
                  <tr class="recent-row" onClick={() => props.onOpenRecent(file.path)}>
                    <td>{file.name}</td>
                    <td>{formatOpenedAt(file.openedAtMs)}</td>
                    <td class="recent-path-cell">{file.path}</td>
                  </tr>
                )}
              </For>
            </Show>
          </tbody>
        </table>
      </section>
    </main>
  );
}
