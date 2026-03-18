import { WebviewWindow } from "@tauri-apps/api/webviewWindow";

const DETACHED_WINDOW_PREFIX = "doc-";

export type DetachedWindowEntry = {
  label: string;
  sessionId: string;
  filePath: string;
  fileName: string;
  updatedAtMs: number;
};

type LaunchContext = {
  mode: "main" | "detached";
  sessionId: string | null;
  windowLabel: string | null;
};

const hashString = (value: string) => {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }

  return hash.toString(36);
};

const sanitizeLabelSegment = (value: string) =>
  value
    .toLowerCase()
    .replace(/[^a-z0-9:_/-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 24) || "doc";

export const buildDetachedWindowLabel = (filePath: string) => {
  const parts = filePath.split(/[\\/]/);
  const fileName = parts[parts.length - 1] ?? "document";
  return `${DETACHED_WINDOW_PREFIX}${sanitizeLabelSegment(fileName)}-${hashString(filePath)}`;
};

export const buildDetachedWindowUrl = (params: Record<string, string | null | undefined>) => {
  const next = new URL(
    "index.html",
    typeof window === "undefined" ? "https://tauri.localhost/" : window.location.href,
  );
  next.hash = "";

  for (const [key, value] of Object.entries(params)) {
    if (!value) {
      continue;
    }
    next.searchParams.set(key, value);
  }

  return `index.html${next.search}`;
};

const waitForWindowCreation = (windowRef: WebviewWindow) =>
  new Promise<void>((resolve, reject) => {
    let settled = false;

    const complete = (fn: () => void) => {
      if (settled) {
        return;
      }
      settled = true;
      fn();
    };

    void windowRef.once("tauri://created", () => complete(resolve));
    void windowRef.once("tauri://error", (event) =>
      complete(() => reject(new Error(String(event.payload ?? "Could not create window")))),
    );
  });

export const createDetachedWindow = async (target: {
  sessionId: string;
  filePath: string;
  fileName: string;
  windowLabel: string;
}) => {
  const existingWindow = await WebviewWindow.getByLabel(target.windowLabel);
  if (existingWindow) {
    await existingWindow.show();
    await existingWindow.setFocus();
    return { label: target.windowLabel, reused: true };
  }

  const detachedWindow = new WebviewWindow(target.windowLabel, {
    url: buildDetachedWindowUrl({
      mode: "detached",
      sessionId: target.sessionId,
      windowLabel: target.windowLabel,
    }),
    title: `${target.fileName} - Tex`,
    width: 1220,
    height: 880,
    minWidth: 940,
    minHeight: 700,
    center: true,
  });

  await waitForWindowCreation(detachedWindow);
  return { label: target.windowLabel, reused: false };
};

export const getLaunchContext = (): LaunchContext => {
  if (typeof window === "undefined") {
    return { mode: "main", sessionId: null, windowLabel: null };
  }

  const url = new URL(window.location.href);
  return {
    mode: url.searchParams.get("mode") === "detached" ? "detached" : "main",
    sessionId: url.searchParams.get("sessionId"),
    windowLabel: url.searchParams.get("windowLabel"),
  };
};
