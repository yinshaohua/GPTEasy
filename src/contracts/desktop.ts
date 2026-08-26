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

export interface DesktopFailure {
  category: string;
  messageId: string;
}

export async function getDesktopSnapshot(): Promise<DesktopSnapshot> {
  if (isBrowserPreview()) return previewSnapshot;
  return parseDesktopSnapshot(await invoke<unknown>("get_desktop_snapshot"));
}

export async function startDesktopApplication(): Promise<DesktopSnapshot> {
  if (isBrowserPreview()) throw previewFailure;
  return parseDesktopSnapshot(await invoke<unknown>("start_desktop_application"));
}

export async function restartDesktopApplication(
  expectedRoots: DesktopIdentity[],
): Promise<DesktopSnapshot> {
  if (isBrowserPreview()) throw previewFailure;
  return parseDesktopSnapshot(await invoke<unknown>("restart_desktop_application", {
    expectedRoots,
  }));
}

export function asDesktopFailure(error: unknown): DesktopFailure {
  if (
    typeof error === "object"
    && error !== null
    && "category" in error
    && "messageId" in error
    && typeof error.category === "string"
    && typeof error.messageId === "string"
  ) {
    return error as DesktopFailure;
  }
  return previewFailure;
}

function parseDesktopSnapshot(value: unknown): DesktopSnapshot {
  if (typeof value !== "object" || value === null) throw previewFailure;
  const candidate = value as Partial<DesktopSnapshot>;
  if (
    !["running", "stopped", "unknown"].includes(candidate.status ?? "")
    || !["start", "restart", "unavailable"].includes(candidate.action ?? "")
    || typeof candidate.messageId !== "string"
    || !Array.isArray(candidate.roots)
    || !candidate.roots.every(isDesktopIdentity)
  ) {
    throw previewFailure;
  }
  return candidate as DesktopSnapshot;
}

function isDesktopIdentity(value: unknown): value is DesktopIdentity {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<DesktopIdentity>;
  return candidate.role === "desktop"
    && Number.isSafeInteger(candidate.pid)
    && Number.isSafeInteger(candidate.startedAtEpochMillis);
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
