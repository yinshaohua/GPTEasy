import { invoke } from "@tauri-apps/api/core";

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
  | "verification_expired"
  | "state_unavailable";

export interface ProviderFailure {
  category: ProviderFailureCategory;
  messageId: string;
}

export interface ModelDiscovery {
  normalizedBaseUrl: string;
  models: string[];
}

export interface ProviderValidationReceipt {
  validationId: string;
  normalizedBaseUrl: string;
  defaultModel: string;
  combinationFingerprint: string;
  verifiedAtEpochSeconds: number;
}

export interface ProviderSummary {
  id: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  verifiedAtEpochSeconds: number;
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

export function discardProviderValidation(validationId: string): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("discard_provider_validation", { validationId });
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
