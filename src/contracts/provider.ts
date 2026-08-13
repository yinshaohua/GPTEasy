import { invoke } from "@tauri-apps/api/core";
import type { ConfigChangeResult, RestartDecision } from "./environment";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ProviderFailureCategory =
  | "security_policy"
  | "transport"
  | "cancelled"
  | "response_header_timeout"
  | "first_event_timeout"
  | "stream_idle_timeout"
  | "overall_timeout"
  | "authentication"
  | "rate_limit"
  | "model_discovery"
  | "streaming"
  | "responses_protocol"
  | "tool_call"
  | "tool_result"
  | "invalid_input"
  | "duplicate_name"
  | "provider_not_found"
  | "current_provider_protected"
  | "save_and_apply_required"
  | "save_and_apply_failed"
  | "clipboard_unavailable"
  | "verification_expired"
  | "state_unavailable";

export interface ProviderFailure {
  category: ProviderFailureCategory;
  messageId: string;
}

export interface ModelDiscovery {
  requestedBaseUrl: string;
  normalizedBaseUrl: string;
  models: string[];
}

export interface ProviderValidationReceipt {
  validationId: string;
  requestedBaseUrl: string;
  normalizedBaseUrl: string;
  defaultModel: string;
  combinationFingerprint: string;
  verifiedAtEpochSeconds: number;
}

export type ProviderValidationStage =
  | "models_confirmed"
  | "responses_stream"
  | "tool_round_trip";

export interface ProviderValidationProgress {
  requestId: string;
  stage: ProviderValidationStage;
}

export function onProviderSwitchRequested(
  handler: (providerId: string) => void,
): Promise<UnlistenFn> {
  if (isBrowserPreview()) return Promise.resolve(() => undefined);
  return listen<string>("provider-switch-requested", (event) => handler(event.payload));
}

export interface ProviderSummary {
  id: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  verifiedAtEpochSeconds: number;
  isCurrent: boolean;
  recommendationId?: "dayway" | null;
  hasRecommendationUpdate?: boolean;
}

export interface ProviderRevalidationResult {
  provider: ProviderSummary;
  validationReceipt: ProviderValidationReceipt | null;
}

export interface ProviderApiKey {
  value: string;
}

export function listProviders(): Promise<ProviderSummary[]> {
  if (isBrowserPreview()) return Promise.resolve([]);
  return invoke<ProviderSummary[]>("list_providers");
}

export function discoverProviderModels(
  requestId: string,
  baseUrl: string,
  apiKey: string,
): Promise<ModelDiscovery> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ModelDiscovery>("discover_provider_models", {
    requestId,
    input: { baseUrl, apiKey },
  });
}

export function discoverProviderModelsForUpdate(
  requestId: string,
  providerId: string,
  baseUrl: string,
  apiKey: string | null,
): Promise<ModelDiscovery> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ModelDiscovery>("discover_provider_models_for_update", {
    requestId,
    input: { providerId, baseUrl, apiKey },
  });
}

export function validateProvider(
  requestId: string,
  baseUrl: string,
  apiKey: string,
  defaultModel: string,
): Promise<ProviderValidationReceipt> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderValidationReceipt>("validate_provider", {
    requestId,
    input: { baseUrl, apiKey, defaultModel },
  });
}

export function validateProviderUpdate(
  requestId: string,
  providerId: string,
  baseUrl: string,
  apiKey: string | null,
  defaultModel: string,
): Promise<ProviderValidationReceipt> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderValidationReceipt>("validate_provider_update", {
    requestId,
    input: { providerId, baseUrl, apiKey, defaultModel },
  });
}

export function revalidateProvider(
  requestId: string,
  providerId: string,
): Promise<ProviderRevalidationResult> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderRevalidationResult>("revalidate_provider", { requestId, providerId });
}

export function confirmProviderValidationBaseUrl(
  validationId: string,
  baseUrl: string,
): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("confirm_provider_validation_base_url", { validationId, baseUrl });
}

export function cancelProviderRequest(requestId: string): Promise<boolean> {
  if (isBrowserPreview()) return Promise.resolve(true);
  return invoke<boolean>("cancel_provider_request", { requestId });
}

export function saveVerifiedProvider(
  validationId: string,
  name: string,
): Promise<ProviderSummary> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderSummary>("save_verified_provider", { validationId, name });
}

export function saveDaywayProvider(
  validationId: string,
  confirmNameConflict = false,
): Promise<ProviderSummary> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderSummary>("save_dayway_provider", { validationId, confirmNameConflict });
}

export function openDaywayWebsite(): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("open_dayway_website");
}

export function renameProvider(providerId: string, name: string): Promise<ProviderSummary> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderSummary>("rename_provider", { providerId, name });
}

export function saveProviderUpdate(
  validationId: string,
  providerId: string,
  name: string,
): Promise<ProviderSummary> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderSummary>("save_provider_update", { validationId, providerId, name });
}

export function saveAndApplyProviderUpdate(
  validationId: string,
  providerId: string,
  name: string,
  restartDecision: RestartDecision,
): Promise<{ provider: ProviderSummary; configChange: ConfigChangeResult }> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<{ provider: ProviderSummary; configChange: ConfigChangeResult }>("save_and_apply_provider_update", {
    validationId,
    providerId,
    name,
    restartDecision,
  });
}

export function deleteProvider(providerId: string): Promise<void> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<void>("delete_provider", { providerId });
}

export function reorderProviders(providerIds: string[]): Promise<ProviderSummary[]> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderSummary[]>("reorder_providers", { providerIds });
}

export function revealProviderApiKey(providerId: string): Promise<ProviderApiKey> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<ProviderApiKey>("reveal_provider_api_key", { providerId });
}

export function copyProviderApiKey(providerId: string): Promise<void> {
  if (isBrowserPreview()) return Promise.reject(previewFailure);
  return invoke<void>("copy_provider_api_key", { providerId });
}

export function discardProviderValidation(validationId: string): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("discard_provider_validation", { validationId });
}

export function onProviderValidationProgress(
  handler: (progress: ProviderValidationProgress) => void,
): Promise<UnlistenFn> {
  if (isBrowserPreview()) return Promise.resolve(() => undefined);
  return listen<ProviderValidationProgress>("provider-validation-progress", (event) => {
    handler(event.payload);
  });
}

export function asProviderFailure(error: unknown): ProviderFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "messageId" in error &&
    typeof error.category === "string" &&
    typeof error.messageId === "string"
  ) {
    return error as ProviderFailure;
  }
  return { category: "transport", messageId: "provider.transport_failed" };
}

function isBrowserPreview(): boolean {
  return import.meta.env.MODE === "development" && !("__TAURI_INTERNALS__" in window);
}

const previewFailure: ProviderFailure = {
  category: "transport",
  messageId: "provider.preview_network_unavailable",
};
