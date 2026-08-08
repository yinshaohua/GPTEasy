import { useCallback, useEffect, useState } from "react";
import {
  CheckCircle2,
  Database,
  FileCode2,
  KeyRound,
  LoaderCircle,
  RefreshCw,
  Server,
  ShieldAlert,
} from "lucide-react";

import ProviderPage from "./ProviderPage";
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
} from "./messages";

type ViewState =
  | { kind: "loading" }
  | { kind: "loaded"; snapshot: StartupSnapshot }
  | { kind: "error" };

type Page = "providers" | "codex";

export default function App() {
  const [state, setState] = useState<ViewState>({ kind: "loading" });
  const [page, setPage] = useState<Page>("providers");
  const [refreshing, setRefreshing] = useState(false);

  const load = useCallback(async (refresh: boolean) => {
    if (refresh) setRefreshing(true);
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

  if (state.kind !== "loaded" || state.snapshot.mode === "blocked") {
    return (
      <Shell>
        {state.kind === "loading" && <LoadingState />}
        {state.kind === "error" && <UnavailableState onRetry={() => void load(true)} />}
        {state.kind === "loaded" && (
          <BlockedState snapshot={state.snapshot} onRetry={() => void load(true)} />
        )}
      </Shell>
    );
  }

  return (
    <div className="app-shell">
      <Sidebar page={page} onNavigate={setPage} />
      <main className="main-content">
        {page === "providers" ? (
          <ProviderPage />
        ) : (
          <CodexStatePage
            snapshot={state.snapshot}
            refreshing={refreshing}
            onRefresh={() => void load(true)}
          />
        )}
      </main>
    </div>
  );
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="应用导航">
        <Brand />
        <div className="sidebar-meta">当前用户</div>
      </aside>
      <main className="main-content">{children}</main>
    </div>
  );
}

function Sidebar({ page, onNavigate }: { page: Page; onNavigate: (page: Page) => void }) {
  return (
    <aside className="sidebar" aria-label="应用导航">
      <Brand />
      <nav className="sidebar-nav">
        <button
          className="nav-item"
          type="button"
          aria-current={page === "providers" ? "page" : undefined}
          onClick={() => onNavigate("providers")}
        >
          <Server size={18} aria-hidden="true" />
          供应商
        </button>
        <button
          className="nav-item"
          type="button"
          aria-current={page === "codex" ? "page" : undefined}
          onClick={() => onNavigate("codex")}
        >
          <FileCode2 size={18} aria-hidden="true" />
          Codex 环境
        </button>
      </nav>
      <div className="sidebar-meta">当前用户</div>
    </aside>
  );
}

function Brand() {
  return (
    <div className="brand">
      <img src="/icon.png" alt="" width="36" height="36" />
      <div>
        <strong>GPTEasy</strong>
        <span>Windows x64</span>
      </div>
    </div>
  );
}

function LoadingState() {
  return (
    <div className="loading-state" role="status">
      <LoaderCircle className="is-spinning" size={22} aria-hidden="true" />
      <span>正在检查本地状态</span>
    </div>
  );
}

function UnavailableState({ onRetry }: { onRetry: () => void }) {
  return (
    <section className="blocked-state" role="alert">
      <ShieldAlert size={26} aria-hidden="true" />
      <div>
        <h2>无法读取启动状态</h2>
        <p>Rust 后端暂时无法返回可信状态。</p>
        <button className="command-button" type="button" onClick={onRetry}>
          <RefreshCw size={17} aria-hidden="true" />
          重新检查
        </button>
      </div>
    </section>
  );
}

function BlockedState({ snapshot, onRetry }: { snapshot: StartupSnapshot; onRetry: () => void }) {
  const reason = snapshot.database.reason;
  const blockReason = snapshot.blockReason;
  const message =
    blockReason === "database_unavailable" && reason
      ? databaseReasonMessages[reason]
      : blockReason
        ? startupBlockMessages[blockReason]
        : "启动状态无法确认。";
  return (
    <section className="blocked-state" role="alert">
      <ShieldAlert size={26} aria-hidden="true" />
      <div>
        <h2>无法安全打开本地状态</h2>
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

function CodexStatePage({
  snapshot,
  refreshing,
  onRefresh,
}: {
  snapshot: StartupSnapshot;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  const contents = snapshot.database.contents;
  return (
    <>
      <header className="page-header">
        <div>
          <h1>启动状态</h1>
          <p>当前用户的 GPTEasy 与 Codex 环境</p>
        </div>
        <button
          className="icon-button"
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          aria-label="重新检查状态"
          title="重新检查状态"
        >
          <RefreshCw className={refreshing ? "is-spinning" : undefined} size={19} />
        </button>
      </header>
      <div className="status-content">
        <section className="summary-band" aria-labelledby="database-summary">
          <CheckCircle2 size={24} aria-hidden="true" />
          <div>
            <h2 id="database-summary">{databaseStatusMessages[snapshot.database.status]}</h2>
            <p>SQLite schema v{snapshot.database.schemaVersion ?? "-"}，启动检查已完成。</p>
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
            <StatusRow label="凭据载体" value={credentialStoreMessages[snapshot.codex.credentialStore]} />
            <StatusRow
              label="文件载体"
              value={credentialFileStatusMessages[snapshot.codex.credentialFileStatus]}
            />
          </dl>
        </section>
      </div>
    </>
  );
}

function StatusRow({ icon, label, value }: { icon?: React.ReactNode; label: string; value: string }) {
  return (
    <div className="status-row">
      <dt>{icon}{label}</dt>
      <dd>{value}</dd>
    </div>
  );
}
