import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath as openNativePath } from "@tauri-apps/plugin-opener";
import type { RootSummary } from "../lib/types";

type EventPayload<T> = {
  event: string;
  payload: T;
};

type Listener<T> = (event: EventPayload<T>) => void;

const listeners = new Map<string, Set<Listener<unknown>>>();

let tauriEventBridgeReady: Promise<void> | null = null;
let updateStatusState: DesktopUpdateStatus = {
  status: "idle",
  message: "Updater is not configured in this Tauri migration yet.",
  info: null,
  error: null,
  updateReady: false,
  lastCheckedAtMs: null,
};

const emit = (event: string, payload: unknown) => {
  const handlers = listeners.get(event);
  if (!handlers) {
    return;
  }

  const wrapped = { event, payload };
  for (const handler of handlers) {
    handler(wrapped);
  }
};

const ensureTauriEventBridge = () => {
  if (tauriEventBridgeReady) {
    return tauriEventBridgeReady;
  }

  tauriEventBridgeReady = Promise.all([
    listen("index-progress", (event) => {
      emit("index-progress", event.payload);
    }),
  ]).then(() => undefined);

  return tauriEventBridgeReady;
};

export type OpenDialogFilter = {
  name?: string;
  extensions?: string[];
};

export type OpenDialogOptions = {
  directory?: boolean;
  multiple?: boolean;
  defaultPath?: string;
  title?: string;
  filters?: OpenDialogFilter[];
};

type AddRootFromDialogResult = {
  canonicalPath: string | null;
  rootsAfter: RootSummary[];
};

export type DesktopUpdateStatus = {
  status: string;
  message: string;
  info: {
    version: string;
    hash: string;
    updateAvailable: boolean;
    updateReady: boolean;
    error: string;
  } | null;
  error: string | null;
  updateReady: boolean;
  lastCheckedAtMs: number | null;
};

export const invokeCore = <T>(
  command: string,
  args?: Record<string, unknown>,
) => invoke<T>("invoke_core_rpc", { command, args: args ?? {} });

const normalizeDialogResult = (raw: unknown): string | string[] | null => {
  if (raw == null) {
    return null;
  }

  if (typeof raw === "string") {
    return raw;
  }

  if (Array.isArray(raw)) {
    return raw.filter((entry): entry is string => typeof entry === "string");
  }

  if (typeof raw === "object") {
    const values = Object.values(raw as Record<string, unknown>).filter(
      (entry): entry is string => typeof entry === "string",
    );

    if (values.length === 0) {
      return null;
    }

    return values;
  }

  return null;
};

export const openDialog = async (options: OpenDialogOptions = {}) => {
  const raw = (await open({
    directory: options.directory,
    multiple: options.multiple,
    defaultPath: options.defaultPath,
    title: options.title,
    filters: options.filters?.map((filter) => ({
      name: filter.name ?? "",
      extensions: filter.extensions ?? [],
    })),
  })) as unknown;

  return normalizeDialogResult(raw);
};

export const addRootFromDialog = async (): Promise<AddRootFromDialogResult> => {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Select folder to index",
  });

  const canonicalPath = normalizeDialogResult(selected);
  if (typeof canonicalPath !== "string" || canonicalPath.trim().length === 0) {
    return {
      canonicalPath: null,
      rootsAfter: await invokeCore<RootSummary[]>("list_roots"),
    };
  }

  const addedPath = await invokeCore<string>("add_root", { path: canonicalPath });
  const rootsAfter = await invokeCore<RootSummary[]>("list_roots");

  return {
    canonicalPath: addedPath,
    rootsAfter,
  };
};

export const openPath = async (path: string) => {
  try {
    await openNativePath(path);
    return true;
  } catch {
    return false;
  }
};

const setUpdateStatus = (next: DesktopUpdateStatus) => {
  updateStatusState = next;
  emit("update-status", next);
};

export const checkForUpdates = async () => {
  const next: DesktopUpdateStatus = {
    ...updateStatusState,
    status: "idle",
    message: "Updater is not configured in this Tauri migration yet.",
    error: null,
    lastCheckedAtMs: Date.now(),
  };
  setUpdateStatus(next);
  return next;
};

export const installUpdateNow = async () => ({ applied: false });

export const getUpdateStatus = async () => updateStatusState;

export const listenEvent = async <T>(
  event: string,
  handler: Listener<T>,
) => {
  if (event === "index-progress") {
    await ensureTauriEventBridge();
  }

  const existing = listeners.get(event);
  if (existing) {
    existing.add(handler as Listener<unknown>);
  } else {
    listeners.set(event, new Set([handler as Listener<unknown>]));
  }

  return () => {
    const bucket = listeners.get(event);
    if (!bucket) {
      return;
    }
    bucket.delete(handler as Listener<unknown>);
    if (bucket.size === 0) {
      listeners.delete(event);
    }
  };
};
