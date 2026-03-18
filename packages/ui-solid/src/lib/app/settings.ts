import { normalizeSlashes } from "../utils";

export const CAPTURE_TARGET_PREFS_KEY = "blockfile.captureTargetsByRoot.v1";
export const SEARCH_FILENAME_ONLY_PREFS_KEY = "blockfile.searchFileNamesOnly.v1";
export const SEARCH_DEBATIFY_ENABLED_PREFS_KEY = "blockfile.searchDebatifyEnabled.v1";

export const normalizeCaptureTargetPath = (value: string) => normalizeSlashes(value).trim();

export const loadStoredBoolean = (key: string, fallback: boolean) => {
  try {
    const raw = localStorage.getItem(key);
    if (raw === "true") return true;
    if (raw === "false") return false;
  } catch {
    // Ignore storage read failures (e.g. restricted storage mode)
  }
  return fallback;
};

export const persistBooleanSetting = (key: string, value: boolean) => {
  try {
    localStorage.setItem(key, value ? "true" : "false");
  } catch {
    // Ignore storage write failures (e.g. restricted storage mode)
  }
};

const loadCaptureTargetPrefs = () => {
  try {
    const raw = localStorage.getItem(CAPTURE_TARGET_PREFS_KEY);
    if (!raw) return {} as Record<string, string>;
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
      return {} as Record<string, string>;
    }
    return parsed as Record<string, string>;
  } catch {
    return {} as Record<string, string>;
  }
};

export const getPersistedCaptureTargetForRoot = (rootPath: string) => {
  const normalizedRoot = normalizeCaptureTargetPath(rootPath);
  const value = loadCaptureTargetPrefs()[normalizedRoot];
  return typeof value === "string" ? normalizeCaptureTargetPath(value) : "";
};

export const persistCaptureTargetForRoot = (rootPath: string, targetPath: string) => {
  const normalizedRoot = normalizeCaptureTargetPath(rootPath);
  const normalizedTarget = normalizeCaptureTargetPath(targetPath);
  if (!normalizedRoot || !normalizedTarget) return;

  const prefs = loadCaptureTargetPrefs();
  prefs[normalizedRoot] = normalizedTarget;
  try {
    localStorage.setItem(CAPTURE_TARGET_PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // Ignore storage write failures (e.g. restricted storage mode)
  }
};
