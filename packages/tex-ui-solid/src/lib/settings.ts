export type TexUserSettings = {
  autoCheckForUpdates: boolean;
};

const SETTINGS_STORAGE_KEY = "tex-user-settings";

const defaultSettings = (): TexUserSettings => ({
  autoCheckForUpdates: true,
});

export const getInitialSettings = (): TexUserSettings => {
  if (typeof window === "undefined") {
    return defaultSettings();
  }

  const defaults = defaultSettings();

  try {
    const raw = window.localStorage.getItem(SETTINGS_STORAGE_KEY);
    if (!raw) {
      return defaults;
    }

    const parsed = JSON.parse(raw) as Partial<TexUserSettings> | null;
    if (!parsed || typeof parsed !== "object") {
      return defaults;
    }

    return {
      autoCheckForUpdates:
        typeof parsed.autoCheckForUpdates === "boolean"
          ? parsed.autoCheckForUpdates
          : defaults.autoCheckForUpdates,
    };
  } catch {
    return defaults;
  }
};

export const persistSettings = (settings: TexUserSettings) => {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(settings));
};
