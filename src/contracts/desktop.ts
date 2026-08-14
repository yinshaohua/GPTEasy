import { invoke } from "@tauri-apps/api/core";

import { isBrowserPreview } from "./browser-preview";

export type DesktopStatus = "running" | "stopped" | "unknown";
export type DesktopAction = "start" | "restart" | "unavailable";

export interface DesktopIdentity {
  role: "desktop";
  pid: number;
  startedAtEpochMillis: number;
}

export interface DesktopSnapshot {
  status: DesktopStatus;
  action: DesktopAction;
  messageId: string;
  roots: DesktopIdentity[];
}

export interface DesktopRestartResult {
  status: "restarted" | "close_timed_out";
  messageId: string;
  desktopIdentities: DesktopIdentity[];
  forceAuthorization: string | null;
}

export interface DesktopFailure {
  category: string;
  messageId: string;
}

export async function getDesktopSnapshot(): Promise<DesktopSnapshot> {
  if (isBrowserPreview()) return previewSnapshot;
  const snapshot = await invoke<unknown>("get_desktop_snapshot");
  if (!isDesktopSnapshot(snapshot)) throw previewFailure;
  return snapshot;
}

export async function startDesktopApplication(): Promise<DesktopSnapshot> {
  if (isBrowserPreview()) return previewSnapshot;
  const snapshot = await invoke<unknown>("start_desktop_application");
  if (!isDesktopSnapshot(snapshot)) throw previewFailure;
  return snapshot;
}

export async function restartDesktopApplication(
  expectedRoots: DesktopIdentity[],
): Promise<DesktopRestartResult> {
  if (isBrowserPreview()) throw previewFailure;
  const result = await invoke<unknown>("restart_desktop_application", { expectedRoots });
  if (!isDesktopRestartResult(result)) throw previewFailure;
  return result;
}

export async function forceRestartDesktopApplication(
  forceAuthorization: string,
): Promise<DesktopRestartResult> {
  if (isBrowserPreview()) throw previewFailure;
  const result = await invoke<unknown>("force_restart_desktop_application", {
    forceAuthorization,
  });
  if (!isDesktopRestartResult(result)) throw previewFailure;
  return result;
}

export function asDesktopFailure(error: unknown): DesktopFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "messageId" in error &&
    typeof error.category === "string" &&
    typeof error.messageId === "string"
  ) {
    return error as DesktopFailure;
  }
  return previewFailure;
}

function isDesktopSnapshot(value: unknown): value is DesktopSnapshot {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<DesktopSnapshot>;
  return (
    ["running", "stopped", "unknown"].includes(candidate.status ?? "") &&
    ["start", "restart", "unavailable"].includes(candidate.action ?? "") &&
    typeof candidate.messageId === "string" &&
    isDesktopIdentities(candidate.roots)
  );
}

function isDesktopRestartResult(value: unknown): value is DesktopRestartResult {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<DesktopRestartResult>;
  return (
    ["restarted", "close_timed_out"].includes(candidate.status ?? "") &&
    typeof candidate.messageId === "string" &&
    isDesktopIdentities(candidate.desktopIdentities) &&
    (typeof candidate.forceAuthorization === "string" || candidate.forceAuthorization === null)
  );
}

function isDesktopIdentities(value: unknown): value is DesktopIdentity[] {
  return (
    Array.isArray(value) &&
    value.every(
      (identity) =>
        typeof identity === "object" &&
        identity !== null &&
        identity.role === "desktop" &&
        Number.isSafeInteger(identity.pid) &&
        Number.isSafeInteger(identity.startedAtEpochMillis),
    )
  );
}

const previewSnapshot: DesktopSnapshot = {
  status: "unknown",
  action: "unavailable",
  messageId: "desktop.platform_unsupported",
  roots: [],
};

const previewFailure: DesktopFailure = {
  category: "state_unavailable",
  messageId: "desktop.state_unavailable",
};
