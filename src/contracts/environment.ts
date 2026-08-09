import { invoke } from "@tauri-apps/api/core";

import type { ProviderSummary } from "./provider";

export type EnvironmentState = "external" | "managed" | "conflict";
export type ArtifactKind = "config" | "credentials";
export type ArtifactAction = "create" | "update";

export interface ArtifactImpact {
  artifact: ArtifactKind;
  action: ArtifactAction;
  fields: string[];
}

export interface EnvironmentSnapshot {
  state: EnvironmentState;
  messageId: string;
  revision: string;
  requiresTakeoverConfirmation: boolean;
  impacts: ArtifactImpact[];
  currentProvider: ProviderSummary | null;
}

export interface EnvironmentFailure {
  category: string;
  messageId: string;
}

export function getEnvironmentSnapshot(): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.resolve(previewSnapshot);
  return invoke<EnvironmentSnapshot>("get_environment_snapshot");
}

export function applyEnvironmentProvider(
  providerId: string,
  confirmTakeover: boolean,
  expectedRevision: string,
): Promise<EnvironmentSnapshot> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<EnvironmentSnapshot>("apply_environment_provider", {
    providerId,
    confirmTakeover,
    expectedRevision,
  });
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

function isBrowserPreview(): boolean {
  return import.meta.env.MODE === "development" && !("__TAURI_INTERNALS__" in window);
}

const previewSnapshot: EnvironmentSnapshot = {
  state: "external",
  messageId: "environment.external",
  revision: "browser-preview",
  requiresTakeoverConfirmation: true,
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
};

const previewFailure: EnvironmentFailure = {
  category: "state_unavailable",
  messageId: "environment.state_unavailable",
};
