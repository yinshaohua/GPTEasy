import { invoke } from "@tauri-apps/api/core";

import { isBrowserPreview } from "./browser-preview";

export type ApplicationMode = "ready" | "blocked";
export type DatabaseStatus = "initialized" | "ready" | "recovered" | "blocked";
export type DatabaseBlockReason =
  | "missing_database"
  | "corrupt_database"
  | "future_schema"
  | "migration_failed"
  | "backup_failed"
  | "recovery_failed"
  | "io_failure";
export type CodexConfigStatus = "missing" | "valid" | "invalid" | "unreadable";
export type LoginStatus = "logged_in" | "not_logged_in" | "unavailable";
export type CredentialStore = "unknown" | "file" | "keyring" | "auto" | "unsupported";
export type CredentialFileStatus = "not_applicable" | "missing" | "present" | "unreadable";
export type StartupBlockReason =
  | "database_unavailable"
  | "codex_config_invalid"
  | "codex_config_unreadable"
  | "pending_config_operation"
  | "managed_config_conflict"
  | "unsupported_credential_store";
export type PendingOperationResolution =
  | "matches_old_state"
  | "matches_new_state"
  | "conflict"
  | "unknown";

export interface PendingConfigOperationSnapshot {
  stage: string;
  oldConfigFingerprint: string | null;
  newConfigFingerprint: string | null;
  oldCredentialsFingerprint: string | null;
  newCredentialsFingerprint: string | null;
}

export interface DatabaseContentsSnapshot {
  providerCount: number;
  hasLastAppliedState: boolean;
  hasPendingConfigOperation: boolean;
  pendingRestart: boolean;
  pendingConfigOperation: PendingConfigOperationSnapshot | null;
}

export interface DatabaseSnapshot {
  status: DatabaseStatus;
  schemaVersion: number | null;
  reason: DatabaseBlockReason | null;
  contents: DatabaseContentsSnapshot | null;
}

export interface CodexSnapshot {
  configStatus: CodexConfigStatus;
  configFingerprint: string | null;
  credentialStore: CredentialStore;
  credentialFileStatus: CredentialFileStatus;
  loginStatus: LoginStatus;
}

export interface StartupSnapshot {
  mode: ApplicationMode;
  messageId: string;
  blockReason: StartupBlockReason | null;
  pendingOperationResolution: PendingOperationResolution | null;
  database: DatabaseSnapshot;
  codex: CodexSnapshot;
}

export function getStartupSnapshot(): Promise<StartupSnapshot> {
  if (isBrowserPreview()) {
    return Promise.resolve(browserPreviewSnapshot);
  }
  return invoke<StartupSnapshot>("get_startup_snapshot");
}

export function refreshStartupSnapshot(): Promise<StartupSnapshot> {
  if (isBrowserPreview()) {
    return Promise.resolve(browserPreviewSnapshot);
  }
  return invoke<StartupSnapshot>("refresh_startup_snapshot");
}

const browserPreviewSnapshot: StartupSnapshot = {
  mode: "ready",
  messageId: "startup.database_initialized",
  blockReason: null,
  pendingOperationResolution: null,
  database: {
    status: "initialized",
    schemaVersion: 1,
    reason: null,
    contents: {
      providerCount: 0,
      hasLastAppliedState: false,
      hasPendingConfigOperation: false,
      pendingRestart: false,
      pendingConfigOperation: null,
    },
  },
  codex: {
    configStatus: "missing",
    configFingerprint: null,
    credentialStore: "unknown",
    credentialFileStatus: "not_applicable",
    loginStatus: "not_logged_in",
  },
};
