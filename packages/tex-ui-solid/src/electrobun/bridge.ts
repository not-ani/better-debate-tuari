import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath as openNativePath } from "@tauri-apps/plugin-opener";
import type {
  RecentFile,
  TexRecoverableSession,
  TexSendInsertMode,
  TexSendRequest,
  TexSendResult,
  TexSessionRouteTarget,
  TexSessionAttachResult,
  TexSessionOpenResult,
  TexSessionSnapshot,
  TexSpeechTargetState,
  TexSessionUpdateArgs,
} from "../lib/types";
import type { DetachedWindowEntry } from "../lib/windowing";

export const RECENT_FILES_STORAGE_KEY = "tex-recent-files";
const RECENT_FILES_LIMIT = 18;

const invokeTauri = <T>(command: string, args: Record<string, unknown>) => invoke<T>(command, args);
const ensureDocxPath = (filePath: string) =>
  filePath.toLowerCase().endsWith(".docx") ? filePath : `${filePath}.docx`;

const readRecentFiles = (): RecentFile[] => {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const raw = window.localStorage.getItem(RECENT_FILES_STORAGE_KEY);
    if (!raw) {
      return [];
    }

    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) {
      return [];
    }

    return parsed.filter((entry): entry is RecentFile => {
      if (!entry || typeof entry !== "object") {
        return false;
      }

      const candidate = entry as Record<string, unknown>;
      return typeof candidate.path === "string" && typeof candidate.name === "string";
    });
  } catch {
    return [];
  }
};

const writeRecentFiles = (entries: RecentFile[]) => {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(RECENT_FILES_STORAGE_KEY, JSON.stringify(entries));
};

const touchRecentFile = (path: string, name: string) => {
  const next: RecentFile[] = [
    {
      path,
      name,
      openedAtMs: Date.now(),
    },
    ...readRecentFiles().filter((entry) => entry.path !== path),
  ].slice(0, RECENT_FILES_LIMIT);

  writeRecentFiles(next);
  return next;
};

export const listRecentFiles = async () => readRecentFiles();

export const rememberRecentSession = (session: Pick<TexSessionSnapshot, "filePath" | "fileName">) =>
  touchRecentFile(session.filePath, session.fileName);

export const openTexSessionDialog = async (
  windowLabel: string,
): Promise<TexSessionOpenResult | null> => {
  const selected = await open({
    directory: false,
    multiple: false,
    title: "Open document",
    filters: [
      {
        name: "Word documents",
        extensions: ["docx"],
      },
    ],
  });

  if (typeof selected !== "string" || selected.trim().length === 0) {
    return null;
  }

  return openTexSessionFromFile(selected, windowLabel);
};

export const openTexSessionFromFile = async (filePath: string, windowLabel: string) =>
  invokeTauri<TexSessionOpenResult>("tex_open_session_from_file", {
    args: { filePath, windowLabel },
  });

export const createTexSessionAtPath = async (filePath: string, windowLabel: string) =>
  invokeTauri<TexSessionOpenResult>("tex_create_session_at_path", {
    args: { filePath: ensureDocxPath(filePath), windowLabel },
  });

export const createTexSessionDialog = async (
  windowLabel: string,
): Promise<TexSessionOpenResult | null> => {
  const selected = await save({
    title: "New document",
    defaultPath: "Untitled.docx",
    filters: [
      {
        name: "Word documents",
        extensions: ["docx"],
      },
    ],
  });

  if (typeof selected !== "string" || selected.trim().length === 0) {
    return null;
  }

  return createTexSessionAtPath(selected, windowLabel);
};

const speechDefaultPath = () => {
  const now = new Date();
  const stamp = [
    now.getFullYear(),
    String(now.getMonth() + 1).padStart(2, "0"),
    String(now.getDate()).padStart(2, "0"),
  ].join("-");
  const time = [
    String(now.getHours()).padStart(2, "0"),
    String(now.getMinutes()).padStart(2, "0"),
  ].join("");
  return `Speech ${stamp} ${time}.docx`;
};

export const createTexSpeechSessionDialog = async (
  windowLabel: string,
): Promise<TexSessionOpenResult | null> => {
  const selected = await save({
    title: "New speech document",
    defaultPath: speechDefaultPath(),
    filters: [
      {
        name: "Word documents",
        extensions: ["docx"],
      },
    ],
  });

  if (typeof selected !== "string" || selected.trim().length === 0) {
    return null;
  }

  return createTexSessionAtPath(selected, windowLabel);
};

export const attachTexSession = async (
  sessionId: string,
  windowLabel: string,
  requestRole: "writer" | "observer",
) =>
  invokeTauri<TexSessionAttachResult>("tex_attach_session", {
    args: { sessionId, windowLabel, requestRole },
  });

export const updateTexSession = async (args: TexSessionUpdateArgs) =>
  invokeTauri<TexSessionSnapshot>("tex_update_session", { args });

export const saveTexSession = async (sessionId: string, windowLabel: string) =>
  invokeTauri<TexSessionSnapshot>("tex_save_session", {
    args: { sessionId, windowLabel },
  });

export const prepareTexPopout = async (
  sessionId: string,
  fromWindowLabel: string,
  toWindowLabel: string,
) =>
  invokeTauri<{
    sessionId: string;
    filePath: string;
    fileName: string;
    windowLabel: string;
  }>("tex_prepare_popout", {
    args: { sessionId, fromWindowLabel, toWindowLabel },
  });

export const releaseTexSession = async (
  sessionId: string,
  windowLabel: string,
  discardUnsaved: boolean,
) =>
  invokeTauri<void>("tex_release_session", {
    args: { sessionId, windowLabel, discardUnsaved },
  });

export const listTexRecoverableSessions = async () =>
  invokeTauri<TexRecoverableSession[]>("tex_list_recoverable_sessions", {});

export const discardTexRecoverableSession = async (sessionId: string) =>
  invokeTauri<void>("tex_discard_recoverable_session", {
    args: { sessionId },
  });

export const listTexDetachedWindows = async () =>
  invokeTauri<DetachedWindowEntry[]>("tex_list_detached_windows", {});

export const listTexOpenSessions = async () =>
  invokeTauri<TexSessionRouteTarget[]>("tex_list_open_sessions", {});

export const getActiveSpeechTarget = async () =>
  invokeTauri<TexSpeechTargetState>("tex_get_active_speech_target", {});

export const setActiveSpeechTarget = async (targetSessionId: string | null) =>
  invokeTauri<TexSpeechTargetState>("tex_set_active_speech_target", {
    args: { targetSessionId },
  });

export const sendToTexSession = async (request: TexSendRequest) =>
  invokeTauri<TexSendResult>("tex_send_to_session", { args: request });

export const listenTexEvent = async <T>(
  event: string,
  handler: (payload: T) => void,
) =>
  listen<T>(event, (eventPayload) => {
    handler(eventPayload.payload);
  });

export const openPath = async (path: string) => {
  try {
    await openNativePath(path);
    return true;
  } catch {
    return false;
  }
};
