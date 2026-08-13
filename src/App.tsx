import { useCallback, useEffect, useState } from "react";
import {
  CirclePlay,
  LoaderCircle,
  MessageSquare,
  RefreshCw,
  RotateCw,
  Server,
  ShieldAlert,
} from "lucide-react";

import ProviderPage from "./ProviderPage";
import {
  asDesktopFailure,
  forceRestartDesktopApplication,
  getDesktopSnapshot,
  restartDesktopApplication,
  startDesktopApplication,
  type DesktopIdentity,
  type DesktopSnapshot,
} from "./contracts/desktop";
import {
  getStartupSnapshot,
  refreshStartupSnapshot,
  type StartupSnapshot,
} from "./contracts/startup";
import {
  databaseReasonMessages,
  desktopMessages,
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

type DesktopDialogState =
  | { kind: "restart"; roots: DesktopIdentity[] }
  | { kind: "force"; authorization: string };

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
  const [dialog, setDialog] = useState<DesktopDialogState | null>(null);
  const [feedback, setFeedback] = useState<string | null>(null);

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
    if (!window.confirm(desktopMessages.startConfirmation)) return;
    setDesktop({ kind: "loading" });
    try {
      setDesktop({ kind: "loaded", snapshot: await startDesktopApplication() });
    } catch (error) {
      setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
    }
  };

  const finishRestart = async () => {
    try {
      setDesktop({ kind: "loaded", snapshot: await getDesktopSnapshot() });
      setFeedback("desktop.restart_succeeded");
    } catch (error) {
      setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
    }
  };

  const confirmRestart = async () => {
    if (!dialog || dialog.kind !== "restart") return;
    const previousRoots = dialog.roots;
    setDialog(null);
    setFeedback(null);
    setDesktop({ kind: "loading" });
    try {
      const result = await restartDesktopApplication(previousRoots);
      if (result.status === "close_timed_out") {
        if (!result.forceAuthorization) {
          setDesktop({ kind: "error", messageId: "desktop.state_unavailable" });
          return;
        }
        setDesktop({
          kind: "loaded",
          snapshot: {
            status: "running",
            action: "restart",
            messageId: result.messageId,
            roots: previousRoots,
          },
        });
        setDialog({ kind: "force", authorization: result.forceAuthorization });
        return;
      }
      await finishRestart();
    } catch (error) {
      setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
    }
  };

  const confirmForceRestart = async () => {
    if (!dialog || dialog.kind !== "force") return;
    const forceAuthorization = dialog.authorization;
    setDialog(null);
    setFeedback(null);
    setDesktop({ kind: "loading" });
    try {
      await forceRestartDesktopApplication(forceAuthorization);
      await finishRestart();
    } catch (error) {
      setDesktop({ kind: "error", messageId: asDesktopFailure(error).messageId });
    }
  };

  const requestDesktopAction = () => {
    if (desktop.kind !== "loaded") return;
    if (desktop.snapshot.action === "start") {
      void startDesktop();
      return;
    }
    if (desktop.snapshot.action === "restart") {
      setFeedback(null);
      setDialog({ kind: "restart", roots: desktop.snapshot.roots });
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
      <DesktopCommand state={desktop} feedback={feedback} onAction={requestDesktopAction} />
      <div className="sidebar-meta">当前用户</div>
      {dialog && (
        <DesktopRestartDialog
          state={dialog}
          onCancel={() => setDialog(null)}
          onConfirm={() =>
            dialog.kind === "restart" ? void confirmRestart() : void confirmForceRestart()
          }
        />
      )}
    </aside>
  );
}

function DesktopCommand({
  state,
  feedback,
  onAction,
}: {
  state: DesktopViewState;
  feedback: string | null;
  onAction: () => void;
}) {
  const snapshot = state.kind === "loaded" ? state.snapshot : null;
  const running = snapshot?.status === "running";
  const enabled = snapshot?.action === "start" || snapshot?.action === "restart";
  const label = running ? desktopMessages.restartLabel : desktopMessages.startLabel;
  const messageId = feedback ?? (state.kind === "error" ? state.messageId : snapshot?.messageId);
  const reason = messageId ? desktopMessages.byId[messageId] : null;

  return (
    <div className="sidebar-command-area">
      <button
        className="sidebar-command"
        type="button"
        disabled={!enabled || state.kind === "loading"}
        onClick={onAction}
      >
        {state.kind === "loading" ? (
          <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
        ) : (
          running ? (
            <RotateCw size={17} aria-hidden="true" />
          ) : (
            <CirclePlay size={17} aria-hidden="true" />
          )
        )}
        <span>{label}</span>
      </button>
      {reason && <span className="sidebar-command-reason">{reason}</span>}
    </div>
  );
}

function DesktopRestartDialog({
  state,
  onCancel,
  onConfirm,
}: {
  state: DesktopDialogState;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const force = state.kind === "force";
  const title = force ? "桌面版未能正常关闭" : "重启 ChatGPT/Codex";
  const rootPids = state.kind === "restart" ? state.roots.map((root) => root.pid) : [];

  return (
    <div className="dialog-backdrop">
      <section
        className="confirmation-dialog desktop-restart-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="desktop-restart-dialog-title"
      >
        <h2 id="desktop-restart-dialog-title">{title}</h2>
        {force ? (
          <p>正常关闭已超时。强制关闭会立即中断正在运行的任务。</p>
        ) : (
          <>
            <p>将先请求以下程序正常关闭，然后通过 Windows 重新激活并核实新进程：</p>
            <ul>
              {rootPids.map((pid) => (
                <li key={pid}>OpenAI 官方 ChatGPT/Codex 桌面版（PID {pid}）</li>
              ))}
            </ul>
            <p>正在运行的任务可能中断。Codex CLI 不会关闭，请在原终端退出并重新运行。</p>
          </>
        )}
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onCancel} autoFocus>
            取消
          </button>
          <button className={force ? "danger-button" : "command-button"} type="button" onClick={onConfirm}>
            {force ? "强制关闭并重启" : "确认重启"}
          </button>
        </div>
      </section>
    </div>
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
