import { useCallback, useEffect, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  Database,
  FileCode2,
  KeyRound,
  LoaderCircle,
  RefreshCw,
  Save,
} from "lucide-react";

import {
  applyEnvironmentProvider,
  asEnvironmentFailure,
  getEnvironmentSnapshot,
  type EnvironmentFailure,
  type EnvironmentSnapshot,
} from "./contracts/environment";
import { listProviders, type ProviderSummary } from "./contracts/provider";
import type { StartupSnapshot } from "./contracts/startup";
import {
  codexConfigMessages,
  credentialFileStatusMessages,
  credentialStoreMessages,
  databaseStatusMessages,
  loginStatusMessages,
  providerFailureMessages,
} from "./messages";

type ViewState =
  | { kind: "loading" }
  | { kind: "error" }
  | { kind: "loaded"; snapshot: EnvironmentSnapshot };

const stateLabels = {
  external: "外部配置",
  managed: "已由 GPTEasy 管理",
  conflict: "管理冲突",
} as const;

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
};

export default function EnvironmentPage({ startup }: { startup: StartupSnapshot }) {
  const [view, setView] = useState<ViewState>({ kind: "loading" });
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [selectedId, setSelectedId] = useState("");
  const [refreshing, setRefreshing] = useState(false);
  const [applying, setApplying] = useState(false);
  const [failure, setFailure] = useState<EnvironmentFailure | null>(null);

  const load = useCallback(async (refresh: boolean) => {
    if (refresh) setRefreshing(true);
    setFailure(null);
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
    if (
      confirmTakeover &&
      !window.confirm("将替换 config.toml 中的供应商字段和 auth.json 中的 API Key。是否继续？")
    ) {
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
          disabled={refreshing || applying}
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
              <h2 id="environment-summary">{stateLabels[view.snapshot.state]}</h2>
              <p>
                {view.snapshot.currentProvider
                  ? `当前供应商：${view.snapshot.currentProvider.name}`
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
                label="用户配置"
                value={codexConfigMessages[startup.codex.configStatus]}
              />
              <StatusRow
                label="OpenAI 登录"
                value={loginStatusMessages[startup.codex.loginStatus]}
              />
              <StatusRow
                label="凭据载体"
                value={credentialStoreMessages[startup.codex.credentialStore]}
              />
              <StatusRow
                label="文件载体"
                value={credentialFileStatusMessages[startup.codex.credentialFileStatus]}
              />
            </dl>
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
                disabled={applying || providers.length === 0}
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
                disabled={applying || !selectedId}
              >
                {applying ? (
                  <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
                ) : (
                  <Save size={17} aria-hidden="true" />
                )}
                {view.snapshot.requiresTakeoverConfirmation ? "确认接管并应用" : "应用供应商"}
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
    messageId: "environment.external",
    revision: "startup-fallback",
    requiresTakeoverConfirmation: true,
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
  };
}
