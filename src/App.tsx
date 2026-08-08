import { useCallback, useEffect, useState } from "react";
import {
  CheckCircle2,
  Database,
  FileCode2,
  KeyRound,
  LoaderCircle,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import {
  getStartupSnapshot,
  refreshStartupSnapshot,
  type StartupSnapshot,
} from "./contracts/startup";
import {
  codexConfigMessages,
  credentialFileStatusMessages,
  credentialStoreMessages,
  databaseReasonMessages,
  databaseStatusMessages,
  loginStatusMessages,
  pendingResolutionMessages,
  startupBlockMessages,
  accessibilityMessages,
} from "./messages";

type ViewState =
  | { kind: "loading" }
  | { kind: "loaded"; snapshot: StartupSnapshot }
  | { kind: "error" };

export default function App() {
  const [state, setState] = useState<ViewState>({ kind: "loading" });
  const [refreshing, setRefreshing] = useState(false);
  const isBusy = state.kind === "loading" || refreshing;
  const isReady = state.kind === "loaded" && state.snapshot.mode === "ready";

  const load = useCallback(async (refresh: boolean) => {
    if (refresh) {
      setRefreshing(true);
    }
    try {
      const snapshot = refresh
        ? await refreshStartupSnapshot()
        : await getStartupSnapshot();
      setState({ kind: "loaded", snapshot });
    } catch {
      setState({ kind: "error" });
    } finally {
      setRefreshing(false);
    }
  }, []);

  useEffect(() => {
    void load(false);
  }, [load]);

  return (
    <div className="app-shell">
      <a className="skip-link" href="#main-content">
        {accessibilityMessages.skipToMain}
      </a>
      <aside className="sidebar" aria-label="应用导航">
        <div className="brand">
          <img src="/icon.png" alt="" width="36" height="36" />
          <div>
            <strong>GPTEasy</strong>
            <span>Windows x64</span>
          </div>
        </div>
        <nav aria-label={accessibilityMessages.pageNavigation}>
          <ul className="nav-list">
            <li>
              {isReady ? (
                <a className="nav-item" href="#local-state-heading" aria-current="page">
                  <Database size={18} aria-hidden="true" />
                  本地状态
                </a>
              ) : (
                <div className="nav-item" aria-current="page">
                  <Database size={18} aria-hidden="true" />
                  本地状态
                </div>
              )}
            </li>
          </ul>
        </nav>
        <div className="sidebar-meta">当前用户</div>
      </aside>

      <main
        id="main-content"
        className="main-content"
        tabIndex={-1}
        aria-labelledby="page-title"
        aria-busy={isBusy}
      >
        <header className="page-header">
          <div>
            <h1 id="page-title">启动状态</h1>
            <p>当前用户的 GPTEasy 与 Codex 环境</p>
          </div>
          <button
            className="icon-button"
            type="button"
            onClick={() => void load(true)}
            disabled={isBusy}
            aria-label={accessibilityMessages.refresh}
            aria-describedby="refresh-status"
            title={accessibilityMessages.refresh}
          >
            <RefreshCw className={refreshing ? "is-spinning" : undefined} size={19} />
          </button>
        </header>
        <p id="refresh-status" className="sr-only" role="status" aria-live="polite" aria-atomic="true">
          {refreshing ? accessibilityMessages.refreshing : ""}
        </p>

        {state.kind === "loading" && <LoadingState />}
        {state.kind === "error" && <UnavailableState onRetry={() => void load(true)} />}
        {state.kind === "loaded" && state.snapshot.mode === "blocked" && (
          <BlockedState snapshot={state.snapshot} onRetry={() => void load(true)} />
        )}
        {state.kind === "loaded" && state.snapshot.mode === "ready" && (
          <ReadyState snapshot={state.snapshot} />
        )}
      </main>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="loading-state" role="status" aria-live="polite" aria-atomic="true">
      <LoaderCircle className="is-spinning" size={22} aria-hidden="true" />
      <span>正在检查本地状态</span>
    </div>
  );
}

function UnavailableState({ onRetry }: { onRetry: () => void }) {
  return (
    <section className="blocked-state" role="alert" aria-labelledby="startup-unavailable-heading">
      <ShieldAlert size={26} aria-hidden="true" />
      <div>
        <h2 id="startup-unavailable-heading">无法读取启动状态</h2>
        <p>Rust 后端暂时无法返回可信状态。</p>
        <button className="command-button" type="button" onClick={onRetry}>
          <RefreshCw size={17} aria-hidden="true" />
          重新检查
        </button>
      </div>
    </section>
  );
}

function BlockedState({
  snapshot,
  onRetry,
}: {
  snapshot: StartupSnapshot;
  onRetry: () => void;
}) {
  const reason = snapshot.database.reason;
  const blockReason = snapshot.blockReason;
  const message =
    blockReason === "database_unavailable" && reason
      ? databaseReasonMessages[reason]
      : blockReason
        ? startupBlockMessages[blockReason]
        : "启动状态无法确认。";
  return (
    <section className="blocked-state" role="alert" aria-labelledby="startup-blocked-heading">
      <ShieldAlert size={26} aria-hidden="true" />
      <div>
        <h2 id="startup-blocked-heading">无法安全打开本地状态</h2>
        <p>{message}</p>
        {snapshot.pendingOperationResolution && (
          <p className="secondary-note">
            {pendingResolutionMessages[snapshot.pendingOperationResolution]}
          </p>
        )}
        <button className="command-button" type="button" onClick={onRetry}>
          <RefreshCw size={17} aria-hidden="true" />
          重新检查
        </button>
      </div>
    </section>
  );
}

function ReadyState({ snapshot }: { snapshot: StartupSnapshot }) {
  const contents = snapshot.database.contents;
  return (
    <div className="status-content">
      <section
        className="summary-band"
        aria-labelledby="database-summary"
        role="status"
        aria-live="polite"
        aria-atomic="true"
      >
        <CheckCircle2 size={24} aria-hidden="true" />
        <div>
          <h2 id="database-summary">{databaseStatusMessages[snapshot.database.status]}</h2>
          <p>
            SQLite schema v{snapshot.database.schemaVersion ?? "-"}，启动检查已完成。
          </p>
        </div>
      </section>

      <section className="status-section" aria-labelledby="local-state-heading">
        <div className="section-heading">
          <Database size={20} aria-hidden="true" />
          <h2 id="local-state-heading">GPTEasy 本地状态</h2>
        </div>
        <dl className="status-list">
          <StatusRow label="数据库" value={databaseStatusMessages[snapshot.database.status]} />
          <StatusRow
            label="Schema"
            value={snapshot.database.schemaVersion ? `v${snapshot.database.schemaVersion}` : "无法确认"}
          />
          <StatusRow label="访问边界" value="Rust 后端" />
          <StatusRow label="已验证供应商" value={String(contents?.providerCount ?? 0)} />
          <StatusRow
            label="未完成配置操作"
            value={contents?.hasPendingConfigOperation ? "需要协调" : "无"}
          />
          <StatusRow label="待重启" value={contents?.pendingRestart ? "是" : "否"} />
        </dl>
      </section>

      <section className="status-section" aria-labelledby="codex-state-heading">
        <div className="section-heading">
          <FileCode2 size={20} aria-hidden="true" />
          <h2 id="codex-state-heading">Codex 环境</h2>
        </div>
        <dl className="status-list">
          <StatusRow
            icon={<FileCode2 size={17} aria-hidden="true" />}
            label="用户配置"
            value={codexConfigMessages[snapshot.codex.configStatus]}
          />
          <StatusRow
            icon={<KeyRound size={17} aria-hidden="true" />}
            label="OpenAI 登录"
            value={loginStatusMessages[snapshot.codex.loginStatus]}
          />
          <StatusRow
            label="凭据载体"
            value={credentialStoreMessages[snapshot.codex.credentialStore]}
          />
          <StatusRow
            label="文件载体"
            value={credentialFileStatusMessages[snapshot.codex.credentialFileStatus]}
          />
        </dl>
      </section>
    </div>
  );
}

function StatusRow({
  icon,
  label,
  value,
}: {
  icon?: React.ReactNode;
  label: string;
  value: string;
}) {
  return (
    <div className="status-row">
      <dt>
        {icon}
        {label}
      </dt>
      <dd>{value}</dd>
    </div>
  );
}
