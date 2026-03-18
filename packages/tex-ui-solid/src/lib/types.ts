export type {
  OpenDocumentResult,
  RecentFile,
  TexBlock,
  TexRecoverableSession,
  TexDocumentPayload,
  TexRouteHeading,
  TexSendInsertMode,
  TexSendRequest,
  TexSendResult,
  TexSessionAttachResult,
  TexSessionOpenResult,
  TexSessionRouteTarget,
  TexSessionOwnerConflict,
  TexSessionSnapshot,
  TexSessionSummary,
  TexSessionUpdatedEvent,
  TexSpeechTargetState,
  TexSessionUpdateArgs,
  TexTextRun,
} from "../../../tex-shared/src/document";

export type TexUpdateStatus =
  | "idle"
  | "checking"
  | "up-to-date"
  | "update-available"
  | "downloading"
  | "installed"
  | "error";

export type TexUpdateState = {
  status: TexUpdateStatus;
  statusLabel: string;
  headline: string;
  detail: string;
  notes: string | null;
  releaseDate: string | null;
  availableVersion: string | null;
  lastCheckedAtMs: number | null;
  downloadedBytes: number | null;
  contentLength: number | null;
  isChecking: boolean;
  isDownloading: boolean;
  canInstall: boolean;
  installButtonLabel: string;
  error: string | null;
};

export type { TexUserSettings } from "./settings";
