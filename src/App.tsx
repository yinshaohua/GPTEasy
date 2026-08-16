import { useCallback, useEffect, useState } from "react";
import {
  LoaderCircle,
  RefreshCw,
  ShieldAlert,
} from "lucide-react";

import AppSidebar from "./AppSidebar";
import ProviderPage from "./ProviderPage";
import SessionPage from "./SessionPage";
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

export default function App() {
  const [state, setState] = useState<ViewState>({ kind: "loading" });
  const [page, setPage] = useState<"providers" | "sessions">("providers");

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

  return page === "providers"
    ? <ProviderPage onOpenSessions={() => setPage("sessions")} />
    : <SessionPage onOpenProviders={() => setPage("providers")} />;
}

function Shell({ children }: { children: React.ReactNode }) {
  return (
    <div className="app-shell">
      <AppSidebar />
      <main className="main-content">{children}</main>
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
