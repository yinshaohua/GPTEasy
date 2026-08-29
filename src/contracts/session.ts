import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { isBrowserPreview } from "./browser-preview";

export type SessionAvailabilityStatus =
  | "available"
  | "codex_missing"
  | "incompatible"
  | "initialization_failed"
  | "recovery_failed";

export interface SessionAvailability {
  status: SessionAvailabilityStatus;
  messageId: string;
  codexVersion: string | null;
  mutation: SessionMutationAvailability;
}

export type SessionMutationAvailabilityStatus =
  | "allowed"
  | "unavailable";

export interface SessionMutationAvailability {
  status: SessionMutationAvailabilityStatus;
  messageId: string;
}

export type SessionActualState = "active" | "archived" | "deleted" | "unknown";
export type SessionMutationResultStatus = "succeeded" | "failed";

export interface SessionMutationResult {
  sessionId: string;
  status: SessionMutationResultStatus;
  actualState: SessionActualState;
  messageId: string;
}

export interface SessionQuery {
  requestId: string | null;
  archived: boolean;
  searchTerm: string | null;
  project: string | null;
  modelProvider: string | null;
  cursor: string | null;
  limit: number;
}

export interface SessionSummary {
  id: string;
  forkedFromId?: string | null;
  parentThreadId?: string | null;
  title: string;
  preview: string;
  project: string;
  modelProvider: string;
  source: string;
  createdAt: number;
  updatedAt: number;
}

export interface SessionListPage {
  sessions: SessionSummary[];
  nextCursor: string | null;
}

export type SessionEntryKind = "user" | "assistant" | "tool";

export interface SessionEntry {
  id: string;
  kind: SessionEntryKind;
  label: string;
  content: string;
  output: string | null;
}

export interface SessionDetail extends SessionSummary {
  entries: SessionEntry[];
}

export interface SessionFailure {
  category: string;
  messageId: string;
}

export type VisibilityTargetMode = "openai_login" | "provider" | "unknown";
export type VisibilityAppServerCapability = "available" | "unavailable" | "incompatible";
export type VisibilityExecutionReadiness =
  | "ready"
  | "cli_running"
  | "unknown_consumer"
  | "desktop_running"
  | "app_server_unavailable"
  | "configuration_blocked"
  | "index_schema_unsupported";
export type VisibilityConsumerState = "none" | "desktop_running" | "cli_running" | "unknown";

export interface SessionVisibilityPreview {
  confirmationId: string;
  target: {
    mode: VisibilityTargetMode;
    modelProvider: string;
    environmentRevision: string;
  };
  codexVersion: string | null;
  appServer: VisibilityAppServerCapability;
  schema: {
    status: string;
    database: string;
    variant: string;
  };
  indexPlan: {
    appServerCoordination: number;
    sqliteFallbackEligible: number;
    schemaSkipped: number;
  };
  summary: {
    candidates: number;
    unchanged: number;
    missingIndex: number;
    skipped: number;
    blocked: number;
    encryptedContentRisk: number;
    active: number;
    archived: number;
  };
  readiness: VisibilityExecutionReadiness;
  consumerState: VisibilityConsumerState;
  canExecute: boolean;
  blockers: string[];
  reasons: Array<{ code: string; count: number }>;
}

export interface SessionVisibilityExecutionRequest {
  confirmationId: string;
  target: SessionVisibilityPreview["target"];
}

export interface SessionVisibilityExecutionResult {
  status: "complete" | "partial" | "failed" | "indeterminate";
  succeeded: number;
  retryable: number;
  encryptedContentRisk: number;
  breakdown: {
    appServerCoordinated: number;
    sqliteFallback: number;
    schemaSkipped: number;
    verificationFailed: number;
  };
  schemaVariant: string;
  consumerState: VisibilityConsumerState;
  writesStarted: boolean;
  recoveryRequired: boolean;
  blockCodexRestart: boolean;
  messageId: string;
  diagnosticStage: string;
  errorCode: string;
}

export type PendingSessionVisibilityStatus = "pending" | "running" | "partial" | "blocked";

export interface PendingSessionVisibility {
  targetMode: "provider" | "openai_login";
  modelProvider: string;
  environmentRevision: string;
  status: PendingSessionVisibilityStatus;
  succeeded: number;
  retryable: number;
  diagnosticStage: string;
  errorCode: string;
  updatedAtEpochSeconds: number;
}

export function getSessionVisibilityStatus(): Promise<PendingSessionVisibility | null> {
  if (isBrowserPreview()) return Promise.resolve(previewPendingVisibility);
  return invoke<PendingSessionVisibility | null>("get_session_visibility_status");
}

export function listenSessionVisibilityStatus(
  handler: (status: PendingSessionVisibility | null) => void,
): Promise<() => void> {
  if (isBrowserPreview()) return Promise.resolve(() => undefined);
  return listen<PendingSessionVisibility | null>("session-visibility-status-changed", (event) => {
    handler(event.payload);
  });
}

export function enterSessionManagement(leaseId: string): Promise<SessionAvailability> {
  if (isBrowserPreview()) return Promise.resolve(previewAvailability);
  return invoke<SessionAvailability>("enter_session_management", { leaseId });
}

export function leaveSessionManagement(leaseId: string): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("leave_session_management", { leaseId });
}

export function listSessions(query: SessionQuery): Promise<SessionListPage> {
  if (isBrowserPreview()) return Promise.resolve(previewList(query));
  return invoke<SessionListPage>("list_sessions", { query });
}

export function previewSessionVisibility(): Promise<SessionVisibilityPreview> {
  if (isBrowserPreview()) return Promise.resolve(previewVisibility);
  return invoke<SessionVisibilityPreview>("preview_session_visibility");
}

export function executeSessionVisibility(
  request: SessionVisibilityExecutionRequest,
): Promise<SessionVisibilityExecutionResult> {
  if (isBrowserPreview()) {
    return Promise.resolve({
      status: "partial",
      succeeded: 1,
      retryable: 1,
      encryptedContentRisk: 1,
      breakdown: {
        appServerCoordinated: 0,
        sqliteFallback: 1,
        schemaSkipped: 0,
        verificationFailed: 0,
      },
      schemaVariant: "codex_0_150_1",
      consumerState: "none",
      writesStarted: true,
      recoveryRequired: true,
      blockCodexRestart: false,
      messageId: "session_visibility.repair_partial",
      diagnosticStage: "rollout_replace",
      errorCode: "session_visibility.write_failed",
    });
  }
  return invoke<SessionVisibilityExecutionResult>("execute_session_visibility", { request });
}

export function cancelSessionRequest(requestId: string): Promise<boolean> {
  if (isBrowserPreview()) return Promise.resolve(true);
  return invoke<boolean>("cancel_session_request", { requestId });
}

export function readSession(sessionId: string): Promise<SessionDetail> {
  if (isBrowserPreview()) {
    const detail = previewDetails[sessionId];
    return detail ? Promise.resolve(detail) : Promise.reject(previewFailure);
  }
  return invoke<SessionDetail>("read_session", { sessionId });
}

export function archiveSessions(sessionIds: string[]): Promise<SessionMutationResult[]> {
  if (isBrowserPreview()) return Promise.resolve(previewMutations(sessionIds, "archived"));
  return invoke<SessionMutationResult[]>("archive_sessions", { sessionIds });
}

export function unarchiveSessions(sessionIds: string[]): Promise<SessionMutationResult[]> {
  if (isBrowserPreview()) return Promise.resolve(previewMutations(sessionIds, "active"));
  return invoke<SessionMutationResult[]>("unarchive_sessions", { sessionIds });
}

export function deleteSession(sessionId: string): Promise<SessionMutationResult> {
  if (isBrowserPreview()) return Promise.resolve(previewMutations([sessionId], "deleted")[0]);
  return invoke<SessionMutationResult>("delete_session", { sessionId });
}

export function chooseSessionExportDestination(suggestedTitle: string): Promise<string | null> {
  if (isBrowserPreview()) return Promise.resolve(null);
  return invoke<string | null>("choose_session_export_destination", { suggestedTitle });
}

export function exportSessionMarkdown(detail: SessionDetail, destination: string): Promise<void> {
  if (isBrowserPreview()) return Promise.resolve();
  return invoke<void>("export_session_markdown", { detail, destination });
}

export function asSessionFailure(error: unknown): SessionFailure {
  if (
    typeof error === "object" &&
    error !== null &&
    "category" in error &&
    "messageId" in error &&
    typeof error.category === "string" &&
    typeof error.messageId === "string"
  ) {
    return error as SessionFailure;
  }
  return previewFailure;
}

const previewAvailability: SessionAvailability = {
  status: "available",
  messageId: "session.available",
  codexVersion: "codex-cli 0.147.0",
  mutation: {
    status: "allowed",
    messageId: "session.mutations_allowed",
  },
};

const previewVisibility: SessionVisibilityPreview = {
  confirmationId: "browser-preview-confirmation",
  target: {
    mode: "provider",
    modelProvider: "preview-provider",
    environmentRevision: "preview-revision",
  },
  codexVersion: "codex-cli 0.150.1",
  appServer: "available",
  schema: { status: "supported", database: "state_5.sqlite", variant: "codex_0_150_1" },
  indexPlan: {
    appServerCoordination: 1,
    sqliteFallbackEligible: 1,
    schemaSkipped: 0,
  },
  summary: {
    candidates: 2,
    unchanged: 1,
    missingIndex: 1,
    skipped: 1,
    blocked: 0,
    encryptedContentRisk: 1,
    active: 3,
    archived: 2,
  },
  readiness: "ready",
  consumerState: "none",
  canExecute: true,
  blockers: [],
  reasons: [
    { code: "provider_mismatch", count: 1 },
    { code: "index_missing", count: 1 },
    { code: "excluded_exec", count: 1 },
    { code: "encrypted_content", count: 1 },
  ],
};

const previewPendingVisibility: PendingSessionVisibility = {
  targetMode: "provider",
  modelProvider: "preview-provider",
  environmentRevision: "preview-revision",
  status: "pending",
  succeeded: 0,
  retryable: 0,
  diagnosticStage: "mode_switch",
  errorCode: "session_visibility.desktop_running",
  updatedAtEpochSeconds: 1_786_140_000,
};

const previewSessions: Array<SessionSummary & { archived: boolean }> = [
  {
    id: "preview-session-1",
    title: "供应商切换一致性",
    preview: "检查切换失败后的配置恢复路径",
    project: "C:\\src\\GPTEasy",
    modelProvider: "dayway",
    source: "Codex CLI",
    createdAt: 1_786_820_000,
    updatedAt: 1_786_906_400,
    archived: false,
  },
  {
    id: "preview-session-2",
    title: "Windows 发布检查",
    preview: "核对安装包与当前用户安装合同",
    project: "C:\\src\\GPTEasy",
    modelProvider: "openai",
    source: "IDE",
    createdAt: 1_786_730_000,
    updatedAt: 1_786_820_000,
    archived: false,
  },
  {
    id: "preview-session-3",
    title: "旧版配置调查",
    preview: "归档的兼容性调查记录",
    project: "C:\\src\\compatibility-long-project-name",
    modelProvider: "已移除的供应商",
    source: "ChatGPT/Codex 桌面版",
    createdAt: 1_786_000_000,
    updatedAt: 1_786_100_000,
    archived: true,
  },
];

const previewDetails: Record<string, SessionDetail> = Object.fromEntries(
  previewSessions.map(({ archived, ...session }) => {
    void archived;
    return [
      session.id,
      {
        ...session,
        entries: [
          { id: `${session.id}-user`, kind: "user", label: "用户", content: session.preview, output: null },
          { id: `${session.id}-tool`, kind: "tool", label: "命令", content: "cargo test --workspace", output: "test result: ok" },
          { id: `${session.id}-assistant`, kind: "assistant", label: "助手", content: "检查完成，结果已记录。", output: null },
        ],
      },
    ];
  }),
);

function previewList(query: SessionQuery): SessionListPage {
  const term = query.searchTerm?.trim().toLocaleLowerCase() ?? "";
  const sessions = previewSessions
    .filter((session) => session.archived === query.archived)
    .filter((session) => !term || `${session.title} ${session.preview}`.toLocaleLowerCase().includes(term))
    .filter((session) => !query.project || session.project === query.project)
    .filter((session) => !query.modelProvider || session.modelProvider === query.modelProvider)
    .map(({ archived, ...session }) => {
      void archived;
      return session;
    });
  return { sessions, nextCursor: null };
}

function previewMutations(
  sessionIds: string[],
  actualState: Exclude<SessionActualState, "unknown">,
): SessionMutationResult[] {
  const messageId = actualState === "archived"
    ? "session.archived"
    : actualState === "active"
      ? "session.unarchived"
      : "session.deleted";
  return sessionIds.map((sessionId) => ({
    sessionId,
    status: "succeeded",
    actualState,
    messageId,
  }));
}

const previewFailure: SessionFailure = {
  category: "request_failed",
  messageId: "session.request_failed",
};
