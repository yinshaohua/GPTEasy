import { useCallback, useEffect, useState } from "react";
import {
  LoaderCircle,
  RefreshCw,
  ShieldAlert,
  X,
} from "lucide-react";

import AppSidebar, { type OpenAiSidebarAction, type UpdateSidebarState } from "./AppSidebar";
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
  updateMessages,
} from "./messages";
import {
  checkForUpdates,
  getUpdateSnapshot,
  openUpdateManualDownload,
  initialUpdateSnapshot,
  type UpdateSnapshot,
} from "./contracts/update";
import { listen } from "@tauri-apps/api/event";

type ViewState =
  | { kind: "loading" }
  | { kind: "loaded"; snapshot: StartupSnapshot }
  | { kind: "error" };

export default function App() {
  const [state, setState] = useState<ViewState>({ kind: "loading" });
  const [page, setPage] = useState<"providers" | "sessions">("providers");
  const [sessionVisited, setSessionVisited] = useState(false);
  const [openAiAction, setOpenAiAction] = useState<OpenAiSidebarAction>();
  const [update, setUpdate] = useState<UpdateSnapshot>(initialUpdateSnapshot);
  const [updateDialogOpen, setUpdateDialogOpen] = useState(false);

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

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<UpdateSnapshot>("update-progress", (event) => {
      if (event.payload) setUpdate(event.payload);
    }).then((dispose) => {
      unlisten = dispose;
    });
    void getUpdateSnapshot().then((snapshot) => {
      if (snapshot) setUpdate(snapshot);
    }).catch(() => undefined);
    return () => unlisten?.();
  }, []);

  const updateSidebar: UpdateSidebarState = {
    snapshot: update,
    onOpen: () => setUpdateDialogOpen(true),
  };

  if (state.kind !== "loaded" || state.snapshot.mode === "blocked") {
    return (
      <Shell update={updateSidebar}>
        {state.kind === "loading" && <LoadingState />}
        {state.kind === "error" && <UnavailableState onRetry={() => void load(true)} />}
        {state.kind === "loaded" && (
          <BlockedState snapshot={state.snapshot} onRetry={() => void load(true)} />
        )}
      </Shell>
    );
  }

  return (
    <>
      <div className="app-view" hidden={page !== "providers"}>
        <ProviderPage
          onOpenAiActionChange={setOpenAiAction}
          update={updateSidebar}
          onOpenSessions={() => {
            setSessionVisited(true);
            setPage("sessions");
          }}
        />
      </div>
      {sessionVisited && (
        <div className="app-view" hidden={page !== "sessions"}>
          <SessionPage
            active={page === "sessions"}
            onOpenProviders={() => setPage("providers")}
            openAiAction={openAiAction}
            update={updateSidebar}
          />
        </div>
      )}
      {updateDialogOpen && (
        <UpdateDialog
          snapshot={update}
          onClose={() => setUpdateDialogOpen(false)}
          onCheck={() => void checkForUpdates().then((snapshot) => {
            if (snapshot) setUpdate(snapshot);
          }).catch(() => undefined)}
          onManualDownload={() => void openUpdateManualDownload()}
        />
      )}
    </>
  );
}

function Shell({ children, update }: { children: React.ReactNode; update?: UpdateSidebarState }) {
  return (
    <div className="app-shell">
      <AppSidebar update={update} />
      <main className="main-content">{children}</main>
    </div>
  );
}

function UpdateDialog({
  snapshot,
  onClose,
  onCheck,
  onManualDownload,
}: {
  snapshot: UpdateSnapshot;
  onClose: () => void;
  onCheck: () => void;
  onManualDownload: () => void;
}) {
  const checkedAt = snapshot.checkedAtEpochSeconds
    ? new Date(snapshot.checkedAtEpochSeconds * 1000).toLocaleString("zh-CN")
    : updateMessages.neverChecked;
  const progress = snapshot.progressPercent === null
    ? snapshot.state === "downloading" ? updateMessages.downloading : null
    : `${snapshot.progressPercent}%`;
  return (
    <div className="dialog-backdrop">
      <section className="update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title">
        <header className="update-dialog-header">
          <div>
            <p className="eyebrow">{updateMessages.eyebrow}</p>
            <h2 id="update-dialog-title">{updateMessages.title}</h2>
          </div>
          <button className="icon-button" type="button" aria-label={updateMessages.close} onClick={onClose}>
            <X size={17} aria-hidden="true" />
          </button>
        </header>
        <dl className="update-details">
          <div><dt>{updateMessages.currentVersion}</dt><dd>v{snapshot.currentVersion}</dd></div>
          <div><dt>{updateMessages.lastCheck}</dt><dd>{checkedAt}</dd></div>
          {snapshot.availableVersion && <div><dt>{updateMessages.targetVersion}</dt><dd>v{snapshot.availableVersion}</dd></div>}
        </dl>
        {snapshot.state === "pending" && <p className="update-ready-note">{updateMessages.pendingNote}</p>}
        {snapshot.state === "failed" && <p className="inline-error">{snapshot.errorMessage ?? (snapshot.failureCategory ? updateMessages.errors[snapshot.failureCategory] : updateMessages.errors.check_failed)}</p>}
        {progress && <div className="update-progress" aria-label={updateMessages.progressLabel}><span style={{ width: snapshot.progressPercent === null ? "35%" : `${snapshot.progressPercent}%` }} /><strong>{progress}</strong></div>}
        {snapshot.notes && <p className="secondary-note">{snapshot.notes}</p>}
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onManualDownload}>{updateMessages.manualDownload}</button>
          <button className="command-button" type="button" onClick={onCheck} disabled={snapshot.state === "checking" || snapshot.state === "downloading"}>
            <RefreshCw size={16} aria-hidden="true" />
            {snapshot.state === "failed" ? updateMessages.retry : updateMessages.check}
          </button>
        </div>
      </section>
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
