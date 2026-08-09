import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Database,
  FileCode2,
  KeyRound,
  LoaderCircle,
  LogIn,
  RefreshCw,
  RotateCcw,
  Save,
} from "lucide-react";

import {
  applyEnvironmentProvider,
  asEnvironmentFailure,
  getEnvironmentSnapshot,
  restoreLastEnvironmentConfig,
  switchToOpenAiLogin,
  type EnvironmentFailure,
  type EnvironmentSnapshot,
} from "./contracts/environment";
import { listProviders, type ProviderSummary } from "./contracts/provider";
import type { StartupSnapshot } from "./contracts/startup";
import {
  authenticationModeMessages,
  codexConfigMessages,
  consumerStatusMessages,
  credentialFileStatusMessages,
  credentialStoreMessages,
  databaseStatusMessages,
  loginStatusMessages,
  environmentStateMessages,
  providerFailureMessages,
} from "./messages";

type ViewState =
  | { kind: "loading" }
  | { kind: "error" }
  | { kind: "loaded"; snapshot: EnvironmentSnapshot };

const failureMessages: Record<string, string> = {
  "environment.state_unavailable": "无法读取 Codex 环境状态。",
  "environment.provider_not_found": "所选供应商已不存在，请刷新后重试。",
  "environment.takeover_confirmation_required": "接管外部配置前需要明确确认。",
  "environment.managed_conflict": "管理区块已被外部修改，当前操作已停止。",
  "environment.file_credentials_required": "当前凭据载体不是可安全写入的 auth.json。",
  "environment.config_invalid": "config.toml 无法安全迁移。",
  "environment.credentials_invalid": "auth.json 不是可安全保留字段的 JSON 对象。",
  "environment.backup_failed": "无法创建完整配置备份，未写入任何工件。",
  "environment.concurrent_modification": "Codex 工件刚刚发生变化，请刷新后重试。",
  "environment.artifact_write_failed": "无法安全写入 Codex 工件，旧状态已保留。",
  "environment.rollback_failed": "旧工件恢复未完成，请重新启动 GPTEasy 进行协调。",
  "environment.restore_confirmation_required": "恢复上次配置前需要明确确认。",
  "environment.restore_unavailable": "当前没有可安全恢复的最近配置。",
  "environment.restore_conflict": "受管工件在最近一次修改后发生变化，请先处理管理冲突。",
  "environment.backup_invalid": "最近一次配置备份不完整，无法安全恢复。",
  "environment.operation_interrupted": "配置操作被中断，请重新启动 GPTEasy 完成恢复协调。",
  "environment.mode_switch_confirmation_required": "模式切换前需要明确确认。",
  "environment.openai_login_required": "请先在 Codex 中完成 OpenAI 登录。",
  "environment.openai_login_unavailable": "无法确认 Codex 登录状态，已阻止切换。",
};

const restoreAvailabilityMessages = {
  available: "可恢复到最近一次 GPTEasy 修改前的配置。",
  no_backup: "尚无可恢复的 GPTEasy 配置修改。",
  artifacts_changed: "受管工件在最近一次修改后发生变化，恢复已禁用。",
  invalid_backup: "最近一次配置备份不完整，恢复已禁用。",
  recovery_pending: "恢复协调完成前不能再次恢复。",
} as const;

export default function EnvironmentPage({ startup }: { startup: StartupSnapshot }) {
  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [switchingOpenAi, setSwitchingOpenAi] = useState(false);
  const [failure, setFailure] = useState<EnvironmentFailure | null>(null);
  const [restoreFailure, setRestoreFailure] = useState<EnvironmentFailure | null>(null);

  const load = useCallback(async (refresh: boolean) => {
    if (refresh) setRefreshing(true);
    setFailure(null);
    setRestoreFailure(null);
    try {
      const [environmentResult, providerResult] = await Promise.all([
        getEnvironmentSnapshot(),
        listProviders(),
      ]);
      const snapshot = environmentResult ?? fallbackEnvironment(startup);
      const catalog = providerResult ?? [];
      setView({ kind: "loaded", snapshot });
      setProviders(catalog);
      setSelectedId((current) => {
        if (catalog.some((provider) => provider.id === current)) return current;
        return snapshot.currentProvider?.id ?? catalog[0]?.id ?? "";
      });
    } catch {
      setView({ kind: "error" });
    } finally {
      setRefreshing(false);
    }
  }, [startup]);

  useEffect(() => {
    void load(false);
  }, [load]);

  async function applySelected() {
    if (view.kind !== "loaded" || !selectedId) return;
    const confirmTakeover = view.snapshot.requiresTakeoverConfirmation;
    const confirmation =
      view.snapshot.mode === "openai_login"
        ? "将从 OpenAI 登录模式切换到所选供应商，并更新 config.toml 与 API Key。是否继续？"
        : "将替换 config.toml 中的供应商字段和 auth.json 中的 API Key。是否继续？";
    if (confirmTakeover && !window.confirm(confirmation)) {
      return;
    }
    setApplying(true);
    setFailure(null);
    try {
      const snapshot = await applyEnvironmentProvider(
        selectedId,
        confirmTakeover,
        view.snapshot.revision,
      );
      setView({ kind: "loaded", snapshot });
      setProviders((current) =>
        current.map((provider) => ({
          ...provider,
          isCurrent: provider.id === snapshot.currentProvider?.id,
        })),
      );
    } catch (error) {
      setFailure(asEnvironmentFailure(error));
    } finally {
      setApplying(false);
    }
  }

  async function enableOpenAiLogin() {
    if (view.kind !== "loaded") return;
    const loginStatus = view.snapshot.loginStatus ?? startup.codex.loginStatus;
    if (loginStatus !== "logged_in" || view.snapshot.mode === "openai_login") return;
    if (
      !window.confirm(
        "将移除 GPTEasy 管理的供应商配置；Codex 登录凭据不会被修改。是否继续？",
      )
    ) {
      return;
    }
    setSwitchingOpenAi(true);
    setFailure(null);
    try {
      const snapshot = await switchToOpenAiLogin(true, view.snapshot.revision);
      setView({ kind: "loaded", snapshot });
      setProviders((current) => current.map((provider) => ({ ...provider, isCurrent: false })));
    } catch (error) {
      setFailure(asEnvironmentFailure(error));
    } finally {
      setSwitchingOpenAi(false);
    }
  }

  async function restoreLatest() {
    if (view.kind !== "loaded" || view.snapshot.restoreAvailability !== "available") return;
    if (
      !window.confirm(
        "将把 config.toml 和 auth.json 恢复到最近一次 GPTEasy 修改前的状态。是否继续？",
      )
    ) {
      return;
    }
    setRestoring(true);
    setRestoreFailure(null);
    try {
      const snapshot = await restoreLastEnvironmentConfig(true, view.snapshot.revision);
      setView({ kind: "loaded", snapshot });
      setProviders((current) =>
        current.map((provider) => ({
          ...provider,
          isCurrent: provider.id === snapshot.currentProvider?.id,
        })),
      );
    } catch (error) {
      setRestoreFailure(asEnvironmentFailure(error));
    } finally {
      setRestoring(false);
    }
  }


  const currentLoginStatus =
    view.kind === "loaded" ? (view.snapshot.loginStatus ?? startup.codex.loginStatus) : startup.codex.loginStatus;
  const desktopConsumerStatus =
    view.kind === "loaded" ? (view.snapshot.consumers?.desktop ?? "unknown") : "unknown";
  const cliConsumerStatus =
    view.kind === "loaded" ? (view.snapshot.consumers?.cli ?? "unknown") : "unknown";

  return (
    <>
      <header className="page-header">
        <div>
          <h1>Codex 环境</h1>
          <p>当前用户默认配置与凭据载体</p>
        </div>
        <button
          className="icon-button"
          type="button"
          onClick={() => void load(true)}
          disabled={refreshing || applying || restoring || switchingOpenAi}
          aria-label="重新检查环境"
          title="重新检查环境"
        >
          <RefreshCw className={refreshing ? "is-spinning" : undefined} size={19} />
        </button>
      </header>

      {view.kind === "loading" && (
        <div className="loading-state" role="status">
          <LoaderCircle className="is-spinning" size={22} aria-hidden="true" />
          <span>正在读取 Codex 环境</span>
        </div>
      )}
      {view.kind === "error" && (
        <section className="blocked-state" role="alert">
          <AlertTriangle size={24} aria-hidden="true" />
          <div>
            <h2>无法读取 Codex 环境</h2>
            <button className="command-button" type="button" onClick={() => void load(true)}>
              <RefreshCw size={17} aria-hidden="true" />
              重新检查
            </button>
          </div>
        </section>
      )}
      {view.kind === "loaded" && (
        <div className="status-content">
          <section
            className={`summary-band environment-summary is-${view.snapshot.state}`}
            aria-labelledby="environment-summary"
          >
            {view.snapshot.state === "managed" ? (
              <CheckCircle2 size={24} aria-hidden="true" />
            ) : (
              <AlertTriangle size={24} aria-hidden="true" />
            )}
            <div>
              <h2 id="environment-summary">
                {view.snapshot.mode
                  ? authenticationModeMessages[view.snapshot.mode]
                  : environmentStateMessages[view.snapshot.state]}
              </h2>
              <p>
                {view.snapshot.currentProvider
                  ? `当前供应商：${view.snapshot.currentProvider.name}`
                  : view.snapshot.mode === "openai_login"
                    ? currentLoginStatus === "logged_in"
                      ? "Codex 已有本地 OpenAI 登录凭据。"
                      : currentLoginStatus === "not_logged_in"
                        ? "Codex 的 OpenAI 登录凭据已失效或被外部注销。"
                        : "当前无法确认 Codex 的 OpenAI 登录状态。"
                  : view.snapshot.state === "conflict"
                    ? "配置所有权无法安全确认。"
                    : "尚未建立有效的 GPTEasy 供应商 ID。"}
              </p>
            </div>
          </section>

          <section className="status-section" aria-labelledby="local-status-heading">
            <div className="section-heading">
              <Database size={20} aria-hidden="true" />
              <h2 id="local-status-heading">
                {databaseStatusMessages[startup.database.status]}
              </h2>
            </div>
            <dl className="status-list">
              <StatusRow
                label="当前模式"
                value={
                  view.snapshot.mode
                    ? authenticationModeMessages[view.snapshot.mode]
                    : environmentStateMessages[view.snapshot.state]
                }
              />
              <StatusRow
                label="用户配置"
                value={codexConfigMessages[startup.codex.configStatus]}
              />
              <StatusRow
                label="OpenAI 登录"
                value={loginStatusMessages[currentLoginStatus]}
              />
              <StatusRow
                label="凭据载体"
                value={credentialStoreMessages[startup.codex.credentialStore]}
              />
              <StatusRow
                label="文件载体"
                value={credentialFileStatusMessages[startup.codex.credentialFileStatus]}
              />
              <StatusRow label="桌面 Codex" value={consumerStatusMessages[desktopConsumerStatus]} />
              <StatusRow label="Codex CLI" value={consumerStatusMessages[cliConsumerStatus]} />
              <StatusRow
                label="待重启"
                value={
                  (view.snapshot.pendingRestart ?? startup.database.contents?.pendingRestart)
                    ? "需要重启消费者"
                    : "无"
                }
              />
            </dl>
          </section>

          <section className="status-section" aria-labelledby="authentication-mode-heading">
            <div className="section-heading">
              <LogIn size={20} aria-hidden="true" />
              <h2 id="authentication-mode-heading">认证模式</h2>
            </div>
            <div className="environment-mode-row">
              {view.snapshot.mode === "openai_login" && currentLoginStatus !== "logged_in" && (
                <p className="mode-warning" role="status">
                  {currentLoginStatus === "not_logged_in"
                    ? "OpenAI 登录已在外部失效；当前模式保持不变。"
                    : "无法确认 OpenAI 登录状态；当前模式保持不变。"}
                </p>
              )}
              {view.snapshot.mode !== "openai_login" && currentLoginStatus !== "logged_in" && (
                <p className="mode-warning" role="status">
                  {currentLoginStatus === "not_logged_in"
                    ? "请先在 Codex 中完成 OpenAI 登录。"
                    : "无法确认 Codex 登录状态，已阻止切换。"}
                </p>
              )}
              <button
                className="command-button"
                type="button"
                onClick={() => void enableOpenAiLogin()}
                disabled={
                  applying ||
                  restoring ||
                  switchingOpenAi ||
                  view.snapshot.mode === "openai_login" ||
                  currentLoginStatus !== "logged_in"
                }
              >
                {switchingOpenAi ? (
                  <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
                ) : (
                  <LogIn size={17} aria-hidden="true" />
                )}
                {view.snapshot.mode === "openai_login"
                  ? "当前为 OpenAI 登录模式"
                  : "切换到 OpenAI 登录模式"}
              </button>
            </div>
          </section>

          <section className="status-section" aria-labelledby="impact-heading">
            <div className="section-heading">
              <FileCode2 size={20} aria-hidden="true" />
              <h2 id="impact-heading">替换范围</h2>
            </div>
            <div className="impact-list">
              {view.snapshot.impacts.map((impact) => (
                <div className="impact-row" key={impact.artifact}>
                  {impact.artifact === "config" ? (
                    <FileCode2 size={18} aria-hidden="true" />
                  ) : (
                    <KeyRound size={18} aria-hidden="true" />
                  )}
                  <div>
                    <strong>{impact.artifact === "config" ? "config.toml" : "auth.json"}</strong>
                    <span>{impact.action === "create" ? "将创建" : "将更新"}</span>
                  </div>
                  <code>{impact.fields.join("、")}</code>
                </div>
              ))}
            </div>
          </section>

          <section className="status-section" aria-labelledby="apply-heading">
            <div className="section-heading">
              <Save size={20} aria-hidden="true" />
              <h2 id="apply-heading">应用供应商</h2>
            </div>
            <div className="environment-apply-row">
              <label htmlFor="environment-provider">要应用的供应商</label>
              <select
                id="environment-provider"
                value={selectedId}
                onChange={(event) => setSelectedId(event.target.value)}
                disabled={applying || restoring || switchingOpenAi || providers.length === 0}
              >
                {providers.length === 0 && <option value="">尚无已验证供应商</option>}
                {providers.map((provider) => (
                  <option key={provider.id} value={provider.id}>
                    {provider.name} · {provider.defaultModel}
                  </option>
                ))}
              </select>
              <button
                className="command-button"
                type="button"
                onClick={() => void applySelected()}
                disabled={applying || restoring || switchingOpenAi || !selectedId}
              >
                {applying ? (
                  <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
                ) : (
                  <Save size={17} aria-hidden="true" />
                )}
                {view.snapshot.mode === "openai_login"
                  ? "确认返回供应商模式"
                  : view.snapshot.requiresTakeoverConfirmation
                    ? "确认接管并应用"
                    : "应用供应商"}
              </button>
            </div>
            {failure && (
              <p className="validation-error" role="alert">
                {failureMessages[failure.messageId] ??
                  providerFailureMessages[failure.messageId] ??
                  "Codex 环境未发生变化，请重试。"}
              </p>
            )}
          </section>

          <section className="status-section" aria-labelledby="restore-heading">
            <div className="section-heading">
              <RotateCcw size={20} aria-hidden="true" />
              <h2 id="restore-heading">恢复</h2>
            </div>
            <div className="environment-restore-row">
              <p>{restoreAvailabilityMessages[view.snapshot.restoreAvailability]}</p>
              <button
                className="command-button"
                type="button"
                onClick={() => void restoreLatest()}
                disabled={
                  applying ||
                  restoring ||
                  switchingOpenAi ||
                  view.snapshot.restoreAvailability !== "available"
                }
              >
                {restoring ? (
                  <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
                ) : (
                  <RotateCcw size={17} aria-hidden="true" />
                )}
                恢复上次配置
              </button>
            </div>
            {restoreFailure && (
              <p className="validation-error" role="alert">
                {failureMessages[restoreFailure.messageId] ?? "Codex 环境未发生变化，请重试。"}
              </p>
            )}
          </section>
        </div>
      )}
    </>
  );
}

function StatusRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="status-row">
      <dt>{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}

function fallbackEnvironment(startup: StartupSnapshot): EnvironmentSnapshot {
  return {
    state: "external",
    mode: null,
    messageId: "environment.external",
    revision: "startup-fallback",
    requiresTakeoverConfirmation: true,
    restoreAvailability: "no_backup",
    impacts: [
      {
        artifact: "config",
        action: startup.codex.configStatus === "missing" ? "create" : "update",
        fields: ["model", "model_provider", "model_providers.<provider-id>"],
      },
      {
        artifact: "credentials",
        action: startup.codex.credentialFileStatus === "missing" ? "create" : "update",
        fields: ["auth_mode", "OPENAI_API_KEY"],
      },
    ],
    currentProvider: null,
    loginStatus: startup.codex.loginStatus,
    pendingRestart: startup.database.contents?.pendingRestart ?? false,
    consumers: {
      desktop: "unknown",
      cli: "unknown",
    },
  };
}
