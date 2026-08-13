import { useCallback, useEffect, useState } from "react";
import {
  FileCode2,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  Server,
  ShieldAlert,
} from "lucide-react";

import EnvironmentPage from "./EnvironmentPage";
import ProviderPage from "./ProviderPage";
import {
  getStartupSnapshot,
  refreshStartupSnapshot,
  type StartupSnapshot,
} from "./contracts/startup";
import {
  databaseReasonMessages,
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

  const load = useCallback(async (refresh: boolean) => {
    try {
      const snapshot = refresh
        ? await refreshStartupSnapshot()
        : await getStartupSnapshot();
      setState({ kind: "loaded", snapshot });
    } catch {
      setState({ kind: "error" });
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
          <EnvironmentPage startup={state.snapshot} />
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
          供应商管理
        </button>
        <button className="nav-item" type="button" disabled aria-disabled="true">
          <MessageSquare size={18} aria-hidden="true" />
          <span>会话管理</span>
          <span className="nav-item-note">即将支持</span>
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
