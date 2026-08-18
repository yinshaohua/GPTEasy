import { invoke } from "@tauri-apps/api/core";

import { isBrowserPreview } from "./browser-preview";

export type UpdateState = "idle" | "checking" | "downloading" | "up_to_date" | "pending" | "failed";
export type UpdateFailureCategory = "check_failed" | "manifest_invalid" | "download_failed" | "signature_invalid";

export interface UpdateSnapshot {
  currentVersion: string;
  state: UpdateState;
  availableVersion: string | null;
  notes: string | null;
  publishedAt: string | null;
  checkedAtEpochSeconds: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  progressPercent: number | null;
  failureCategory: UpdateFailureCategory | null;
  errorMessage: string | null;
  manualDownloadUrl: string;
}

export const initialUpdateSnapshot: UpdateSnapshot = {
  currentVersion: "1.0.1",
  state: "idle",
  availableVersion: null,
  notes: null,
  publishedAt: null,
  checkedAtEpochSeconds: null,
  downloadedBytes: 0,
  totalBytes: null,
  progressPercent: null,
  failureCategory: null,
  errorMessage: null,
  manualDownloadUrl: "https://github.com/yinshaohua/GPTEasy/releases/latest",
};

export function getUpdateSnapshot(): Promise<UpdateSnapshot> {
  return isBrowserPreview()
    ? Promise.resolve(initialUpdateSnapshot)
    : invoke<UpdateSnapshot>("get_update_snapshot");
}

export function checkForUpdates(): Promise<UpdateSnapshot> {
  return isBrowserPreview()
    ? Promise.resolve(initialUpdateSnapshot)
    : invoke<UpdateSnapshot>("check_for_updates");
}

export function openUpdateManualDownload(): Promise<void> {
  return isBrowserPreview()
    ? Promise.resolve()
    : invoke<void>("open_update_manual_download");
}
