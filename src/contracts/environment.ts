import { invoke } from "@tauri-apps/api/core";

import { isBrowserPreview } from "./browser-preview";
import type { ProviderSummary } from "./provider";
import type { LoginStatus } from "./startup";

export type EnvironmentState = "external" | "managed" | "conflict";
export type AuthenticationMode = "provider" | "openai_login";
export type ConsumerStatus = "running" | "stopped" | "unknown";
export type ArtifactKind = "config" | "credentials";
export type ArtifactAction = "create" | "update";
export type RestoreAvailability =
  | "available"
  | "no_backup"
  | "artifacts_changed"
  | "invalid_backup"
  | "recovery_pending";

export interface ArtifactImpact {
  artifact: ArtifactKind;
  action: ArtifactAction;
  fields: string[];
}

export interface EnvironmentSnapshot {
  state: EnvironmentState;
  mode: AuthenticationMode | null;
  messageId: string;
  revision: string;
  requiresTakeoverConfirmation: boolean;
  takeoverAvailable: boolean;
  impacts: ArtifactImpact[];
  currentProvider: ProviderSummary | null;
  restoreAvailability: RestoreAvailability;
  restorePreview: {
    artifacts: ArtifactKind[];
    targetMode: AuthenticationMode | null;
    targetProvider: ProviderSummary | null;
  } | null;
  loginStatus: LoginStatus;
  pendingRestart: boolean;
  requiresConsumerConfirmation: boolean;
  consumers: {
    desktop: ConsumerStatus;
    cli: ConsumerStatus;
  };
}

export interface EnvironmentFailure {
  category: string;
  messageId: string;
}

export type WslAvailability =
  | "manageable"
  | "infrastructure"
  | "unsupported_version"
  | "ambiguous"
  | "removed"
  | "unavailable"
  | "default_user_changed"
  | "needs_refresh";

export type WslConfigurationState =
  | "unknown"
  | "none"
  | "current"
  | "updated"
  | "legacy"
  | "provider_missing"
  | "conflict"
  | "busy";

export type WslLifecycleOutcome =
  | "unchanged_running"
  | "stopped_naturally"
  | "still_running";

export interface WslEnvironmentSummary {
  environmentId: string;
  displayName: string;
  commandName: string | null;
  defaultUid: number | null;
  running: boolean;
  availability: WslAvailability;
  currentProvider: ProviderSummary | null;
  actualProviderId: string | null;
  configurationState: WslConfigurationState;
  requiresAttention: boolean;
  pendingRestart: boolean;
  revision: string;
  messageId: string | null;
  reclaimPreview: {
    scope: "preserve_unrelated_toml" | "rebuild_minimal_config";
    fullConfigBackup: boolean;
    authJsonUnchanged: boolean;
    temporarilyStartsDistribution: boolean;
  } | null;
}

export interface WslApplyResult {
  environment: WslEnvironmentSummary;
  pendingRestart: boolean;
  lifecycleOutcome: WslLifecycleOutcome;
}

export interface WslRefreshResult {
  environment: WslEnvironmentSummary;
  lifecycleOutcome: WslLifecycleOutcome;
}

export interface WslLifecycleResult {
  environmentId: string;
  displayName: string;
  outcome: WslLifecycleOutcome;
}

export interface WslFailure {
  category: string;
  messageId: string;
  lifecycleOutcome?: WslLifecycleOutcome;
}

export function getEnvironmentSnapshot(): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.resolve(previewSnapshot);
  return invoke<EnvironmentSnapshot>("get_environment_snapshot");
}

export function applyEnvironmentProvider(
  providerId: string,
  expectedRevision: string,
): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<EnvironmentSnapshot>("apply_environment_provider", {
    providerId,
    expectedRevision,
  });
}

export function forceApplyEnvironmentProvider(
  providerId: string,
  expectedRevision: string,
  confirmRebuild = false,
): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<EnvironmentSnapshot>("force_apply_environment_provider", {
    providerId,
    expectedRevision,
    confirmRebuild,
  });
}

export function restoreLastEnvironmentConfig(
  confirmRestore: boolean,
  expectedRevision: string,
): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<EnvironmentSnapshot>("restore_last_environment_config", {
    confirmRestore,
    expectedRevision,
  });
}

export function switchToOpenAiLogin(
  expectedRevision: string,
): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<EnvironmentSnapshot>("switch_to_openai_login", {
    expectedRevision,
  });
}

export function listWslEnvironments(): Promise<WslEnvironmentSummary[]> {
  if (isBrowserPreview()) return Promise.resolve([]);
  return invoke<WslEnvironmentSummary[] | null>("list_wsl_environments").then((result) => result ?? []);
}

export function applyWslProvider(
  environmentId: string,
  providerId: string,
  expectedRevision: string,
  confirm = true,
): Promise<WslApplyResult> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<WslApplyResult>("apply_wsl_provider", {
    environmentId,
    providerId,
    expectedRevision,
    confirm,
  });
}

export function reclaimWslProvider(
  environmentId: string,
  providerId: string,
  expectedRevision: string,
  confirmReclaim: boolean,
): Promise<WslApplyResult> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<WslApplyResult>("reclaim_wsl_provider", {
    environmentId,
    providerId,
    expectedRevision,
    confirmReclaim,
  });
}

export function refreshWslEnvironment(
  environmentId: string,
  expectedRevision: string,
  authorizeStart: boolean,
): Promise<WslRefreshResult> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<WslRefreshResult>("refresh_wsl_environment", {
    environmentId,
    expectedRevision,
    authorizeStart,
  });
}

export function asWslFailure(error: unknown): WslFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "messageId" in error &&
    typeof error.category === "string" &&
    typeof error.messageId === "string"
  ) {
    return error as WslFailure;
  }
  return { category: "state_unavailable", messageId: "wsl.state_unavailable" };
}

export function asEnvironmentFailure(error: unknown): EnvironmentFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "messageId" in error &&
    typeof error.category === "string" &&
    typeof error.messageId === "string"
  ) {
    return error as EnvironmentFailure;
  }
  return previewFailure;
}

const previewSnapshot: EnvironmentSnapshot = {
  state: "external",
  mode: null,
  messageId: "environment.external",
  revision: "browser-preview",
  requiresTakeoverConfirmation: true,
  takeoverAvailable: true,
  restoreAvailability: "no_backup",
  restorePreview: null,
  impacts: [
    {
      artifact: "config",
      action: "create",
      fields: ["model", "model_provider", "model_providers.<provider-id>"],
    },
    {
      artifact: "credentials",
      action: "create",
      fields: ["auth_mode", "OPENAI_API_KEY"],
    },
  ],
  currentProvider: null,
  loginStatus: "not_logged_in",
  pendingRestart: false,
  requiresConsumerConfirmation: true,
  consumers: {
    desktop: "unknown",
    cli: "unknown",
  },
};

const previewFailure: EnvironmentFailure = {
  category: "state_unavailable",
  messageId: "environment.state_unavailable",
};
