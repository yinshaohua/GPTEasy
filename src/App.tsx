import { useCallback, useEffect, useState } from "react";
import {
  CirclePlay,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  Server,
  ShieldAlert,
} from "lucide-react";

import ProviderPage from "./ProviderPage";
import {
  asDesktopFailure,
  getDesktopSnapshot,
  startDesktopApplication,
  type DesktopSnapshot,
} from "./contracts/desktop";
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

type DesktopViewState =
  | { kind: "loading" }
  | { kind: "loaded"; snapshot: DesktopSnapshot }
  | { kind: "error"; messageId: string };

export default function App() {
  const [state, setState] = useState<ViewState>({ kind: "loading" });

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
      <Sidebar />
      <main className="main-content">
        <ProviderPage />
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

function Sidebar() {
  const [desktop, setDesktop] = useState<DesktopViewState>({ kind: "loading" });

  useEffect(() => {
    let current = true;
    void getDesktopSnapshot()
      .then((snapshot) => {
        if (current) setDesktop({ kind: "loaded", snapshot });
      })
      .catch((error: unknown) => {
        if (current) setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
      });
    return () => {
      current = false;
    };
  }, []);

  const startDesktop = async () => {
    if (desktop.kind !== "loaded" || desktop.snapshot.action !== "start") return;
    if (!window.confirm("将启动 OpenAI 官方 ChatGPT/Codex 桌面版。是否继续？")) return;
    setDesktop({ kind: "loading" });
    try {
      setDesktop({ kind: "loaded", snapshot: await startDesktopApplication() });
    } catch (error) {
      setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
    }
  };

  return (
    <aside className="sidebar" aria-label="应用导航">
      <Brand />
      <nav className="sidebar-nav">
        <button
          className="nav-item"
          type="button"
          aria-current="page"
        >
          <Server size={18} aria-hidden="true" />
          供应商管理
        </button>
        <button className="nav-item" type="button" disabled aria-disabled="true">
          <MessageSquare size={18} aria-hidden="true" />
          <span>会话管理</span>
          <span className="nav-item-note">即将支持</span>
        </button>
      </nav>
      <DesktopCommand state={desktop} onStart={() => void startDesktop()} />
      <div className="sidebar-meta">当前用户</div>
    </aside>
  );
}

function DesktopCommand({
  state,
  onStart,
}: {
  state: DesktopViewState;
  onStart: () => void;
}) {
  const snapshot = state.kind === "loaded" ? state.snapshot : null;
  const running = snapshot?.status === "running";
  const enabled = snapshot?.action === "start";
  const label = running ? "ChatGPT/Codex 正在运行" : "启动 ChatGPT/Codex";
  const messageId = state.kind === "error" ? state.messageId : snapshot?.messageId;
  const reason = messageId ? desktopMessage(messageId) : null;

  return (
    <div className="sidebar-command-area">
      <button
        className="sidebar-command"
        type="button"
        disabled={!enabled || state.kind === "loading"}
        onClick={onStart}
      >
        {state.kind === "loading" ? (
          <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
        ) : (
          <CirclePlay size={17} aria-hidden="true" />
        )}
        <span>{label}</span>
      </button>
      {reason && <span className="sidebar-command-reason">{reason}</span>}
    </div>
  );
}

function desktopMessage(messageId: string): string | null {
  const messages: Record<string, string> = {
    "desktop.identity_untrusted": "无法可靠确认桌面版身份，启动已禁用。",
    "desktop.not_installed": "未发现 OpenAI 官方 ChatGPT/Codex 桌面版。",
    "desktop.ambiguous_installation": "发现多个桌面版候选，无法安全启动。",
    "desktop.discovery_failed": "无法读取桌面版安装信息。",
    "desktop.activation_failed": "Windows 未能激活 ChatGPT/Codex。",
    "desktop.launch_not_observed": "激活后未发现可信的新桌面进程。",
    "desktop.action_unavailable": "桌面版当前不可启动。",
    "desktop.state_unavailable": "无法读取桌面版状态。",
    "desktop.platform_unsupported": "当前平台暂不支持桌面版启动。",
  };
  return messages[messageId] ?? null;
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
