import { For, Show } from "solid-js";
import type { ThemeMode } from "../lib/theme";
import type { TexUpdateState, TexUserSettings } from "../lib/types";

const formatCheckTime = (lastCheckedAtMs: number | null) => {
  if (!lastCheckedAtMs) {
    return "Never checked";
  }

  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(lastCheckedAtMs);
};

const formatByteCount = (value: number | null) => {
  if (value == null || !Number.isFinite(value)) {
    return "Unknown size";
  }

  if (value < 1024) {
    return `${value} B`;
  }

  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KB`;
  }

  if (value < 1024 * 1024 * 1024) {
    return `${(value / (1024 * 1024)).toFixed(1)} MB`;
  }

  return `${(value / (1024 * 1024 * 1024)).toFixed(1)} GB`;
};

type SettingsScreenProps = {
  theme: ThemeMode;
  appVersion: string;
  busy: boolean;
  settings: TexUserSettings;
  updateState: TexUpdateState;
  onBack: () => void;
  onToggleTheme: () => void;
  onToggleAutoCheckForUpdates: () => void;
  onCheckForUpdates: () => void;
  onInstallUpdate: () => void;
};

export default function SettingsScreen(props: SettingsScreenProps) {
  const progressWidth = () => {
    const total = props.updateState.contentLength;
    const downloaded = props.updateState.downloadedBytes;
    if (!total || total <= 0 || downloaded == null) {
      return "0%";
    }

    const ratio = Math.max(0, Math.min(1, downloaded / total));
    return `${Math.round(ratio * 100)}%`;
  };

  const secondaryFacts = () =>
    [
      `Current version ${props.appVersion}`,
      `Last check ${formatCheckTime(props.updateState.lastCheckedAtMs)}`,
      props.updateState.availableVersion ? `Latest version ${props.updateState.availableVersion}` : null,
      props.updateState.releaseDate ? `Published ${props.updateState.releaseDate}` : null,
    ].filter((value): value is string => Boolean(value));

  return (
    <main class="settings-screen">
      <header class="settings-hero">
        <div class="settings-hero-copy">
          <p class="settings-kicker">Tex control room</p>
          <h1>Settings</h1>
          <p>
            Tune the workspace, control update behavior, and keep this build in step with the
            latest signed release.
          </p>
        </div>

        <div class="settings-hero-actions">
          <button class="settings-ghost-button" onClick={props.onBack} type="button">
            Back
          </button>
          <button class="settings-primary-button" onClick={props.onCheckForUpdates} type="button">
            Check now
          </button>
        </div>
      </header>

      <section class="settings-grid" aria-label="Tex settings">
        <article class="settings-card settings-card-feature">
          <div class="settings-card-header">
            <div>
              <p class="settings-card-eyebrow">Updates</p>
              <h2>{props.updateState.headline}</h2>
            </div>
            <span class={`settings-status-pill is-${props.updateState.status}`}>
              {props.updateState.statusLabel}
            </span>
          </div>

          <p class="settings-card-copy">{props.updateState.detail}</p>

          <div class="settings-inline-facts">
            <For each={secondaryFacts()}>{(fact) => <span>{fact}</span>}</For>
          </div>

          <Show when={props.updateState.isDownloading}>
            <div class="settings-progress-shell" aria-label="Download progress">
              <div class="settings-progress-track">
                <div class="settings-progress-bar" style={{ width: progressWidth() }} />
              </div>
              <div class="settings-progress-meta">
                <span>{progressWidth()}</span>
                <span>
                  {formatByteCount(props.updateState.downloadedBytes)} of{" "}
                  {formatByteCount(props.updateState.contentLength)}
                </span>
              </div>
            </div>
          </Show>

          <Show when={props.updateState.notes}>
            <section class="settings-release-notes">
              <p class="settings-card-eyebrow">Release notes</p>
              <p>{props.updateState.notes}</p>
            </section>
          </Show>

          <div class="settings-card-actions">
            <button
              class="settings-secondary-button"
              disabled={props.updateState.isChecking || props.updateState.isDownloading}
              onClick={props.onCheckForUpdates}
              type="button"
            >
              {props.updateState.isChecking ? "Checking..." : "Check for updates"}
            </button>
            <button
              class="settings-primary-button"
              disabled={!props.updateState.canInstall}
              onClick={props.onInstallUpdate}
              type="button"
            >
              {props.updateState.installButtonLabel}
            </button>
          </div>
        </article>

        <article class="settings-card">
          <div class="settings-card-header">
            <div>
              <p class="settings-card-eyebrow">Appearance</p>
              <h2>Reading atmosphere</h2>
            </div>
          </div>

          <div class="settings-option-list">
            <button class="settings-toggle-row" onClick={props.onToggleTheme} type="button">
              <div>
                <strong>{props.theme === "light" ? "Switch to dark mode" : "Switch to light mode"}</strong>
                <span>
                  Flip the writing surface between the bright editorial canvas and the low-light
                  desk lamp palette.
                </span>
              </div>
              <span class="settings-chip">{props.theme === "light" ? "Light" : "Dark"}</span>
            </button>
          </div>
        </article>

        <article class="settings-card">
          <div class="settings-card-header">
            <div>
              <p class="settings-card-eyebrow">Automation</p>
              <h2>Release cadence</h2>
            </div>
          </div>

          <div class="settings-option-list">
            <button
              aria-pressed={props.settings.autoCheckForUpdates}
              class="settings-toggle-row"
              onClick={props.onToggleAutoCheckForUpdates}
              type="button"
            >
              <div>
                <strong>Background update checks</strong>
                <span>
                  Let Tex quietly check signed releases on launch and keep the updater panel warm.
                </span>
              </div>
              <span class="settings-chip">
                {props.settings.autoCheckForUpdates ? "Enabled" : "Disabled"}
              </span>
            </button>
          </div>
        </article>
      </section>
    </main>
  );
}
