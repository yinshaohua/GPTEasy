import { invoke } from "@tauri-apps/api/core";

export type DesktopStatus = "running" | "stopped" | "unknown";
export type DesktopAction = "start" | "unavailable";

export interface DesktopSnapshot {
  status: DesktopStatus;
  action: DesktopAction;
  messageId: string;
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
    ["start", "unavailable"].includes(candidate.action ?? "") &&
    typeof candidate.messageId === "string"
  );
}

function isBrowserPreview(): boolean {
  return import.meta.env.MODE === "development" && !("__TAURI_INTERNALS__" in window);
}

const previewSnapshot: DesktopSnapshot = {
  status: "unknown",
  action: "unavailable",
  messageId: "desktop.platform_unsupported",
};

const previewFailure: DesktopFailure = {
  category: "state_unavailable",
  messageId: "desktop.state_unavailable",
};
