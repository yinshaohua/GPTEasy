import { useCallback, useEffect, useRef, useState } from "react";
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
  installUpdate,
  openUpdateManualDownload,
  openUpdateReleaseNotes,
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
  const [installingUpdate, setInstallingUpdate] = useState(false);
  const [installError, setInstallError] = useState<string | null>(null);
  const [currentProviderName, setCurrentProviderName] = useState<string | null>(null);
  const installInFlight = useRef(false);

  const handleInstall = useCallback(() => {
    if (installInFlight.current) return;
    installInFlight.current = true;
    setUpdateDialogOpen(false);
    setInstallingUpdate(true);
    setInstallError(null);
    void installUpdate().then((snapshot) => {
      if (snapshot) setUpdate(snapshot);
    }).catch((error: { messageId?: string; message_id?: string }) => {
      installInFlight.current = false;
      setInstallingUpdate(false);
      const messageId = error?.messageId ?? error?.message_id;
      setInstallError(messageId === "update.busy"
        ? updateMessages.installBusy
        : updateMessages.errors[(messageId?.replace("update.", "") ?? "") as keyof typeof updateMessages.errors]
          ?? updateMessages.errors.launch_failed);
      setUpdateDialogOpen(true);
    });
  }, []);

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
    installing: installingUpdate,
    onInstall: handleInstall,
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
          onCurrentProviderNameChange={setCurrentProviderName}
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
            currentProviderName={currentProviderName}
            update={updateSidebar}
          />
        </div>
      )}
      {updateDialogOpen && (
        <UpdateDialog
          snapshot={update}
          onClose={() => setUpdateDialogOpen(false)}
          installError={installError}
          installing={installingUpdate}
          onCheck={() => void checkForUpdates().then((snapshot) => {
            setInstallError(null);
            if (snapshot) setUpdate(snapshot);
          }).catch(() => undefined)}
          onManualDownload={() => void openUpdateManualDownload()}
          onOpenReleaseNotes={(url) => void openUpdateReleaseNotes(url)}
          onInstall={handleInstall}
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
  onOpenReleaseNotes,
  onInstall,
  installError,
  installing,
}: {
  snapshot: UpdateSnapshot;
  onClose: () => void;
  onCheck: () => void;
  onManualDownload: () => void;
  onOpenReleaseNotes: (url: string) => void;
  onInstall: () => void;
  installError: string | null;
  installing: boolean;
}) {
  const checkedAt = snapshot.checkedAtEpochSeconds
    ? new Date(snapshot.checkedAtEpochSeconds * 1000).toLocaleString("zh-CN")
    : updateMessages.neverChecked;
  const progress = snapshot.progressPercent === null
    ? snapshot.state === "downloading" ? updateMessages.downloading : null
    : `${snapshot.progressPercent}%`;
  const releaseNotes = snapshot.notes?.split(/\n\s*\n/)[0]?.trim() || null;
  const pending = snapshot.state === "pending";
  const incomplete = snapshot.state === "incomplete";
  return (
    <div className="dialog-backdrop">
      <section className="update-dialog" role="dialog" aria-modal="true" aria-labelledby="update-dialog-title" aria-busy={installing || undefined}>
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
        {pending && <p className="update-ready-note">{updateMessages.pendingNote}</p>}
        {incomplete && <p className="update-ready-note">{updateMessages.incompleteNote}</p>}
        {snapshot.state === "checking" && (
          <p className="update-status-note" role="status">{updateMessages.status.checking}</p>
        )}
        {snapshot.state === "up_to_date" && (
          <p className="update-ready-note" role="status">{updateMessages.status.up_to_date}</p>
        )}
        {snapshot.state === "failed" && <p className="inline-error">{snapshot.errorMessage ?? (snapshot.failureCategory ? updateMessages.errors[snapshot.failureCategory] : updateMessages.errors.check_failed)}</p>}
        {installError && <p className="inline-error" role="alert">{installError}</p>}
        {progress && <div className="update-progress" aria-label={updateMessages.progressLabel}><span style={{ width: snapshot.progressPercent === null ? "35%" : `${snapshot.progressPercent}%` }} /><strong>{progress}</strong></div>}
        {releaseNotes && <p className="secondary-note">{releaseNotes}</p>}
        {pending && snapshot.releaseNotesUrl && (
          <button className="text-link update-release-link" type="button" onClick={() => onOpenReleaseNotes(snapshot.releaseNotesUrl!)}>
            {updateMessages.releaseNotes}
          </button>
        )}
        <div className="update-dialog-actions">
          <button className="secondary-button update-check-button" type="button" onClick={onCheck} disabled={installing || snapshot.state === "checking" || snapshot.state === "downloading"}>
            <RefreshCw size={16} aria-hidden="true" />
            {snapshot.state === "incomplete" ? updateMessages.redownload : snapshot.state === "failed" ? updateMessages.retry : updateMessages.check}
          </button>
          <div className="dialog-actions">
            <button className="secondary-button" type="button" onClick={onManualDownload} disabled={installing}>{updateMessages.manualDownload}</button>
            {pending && <button className="secondary-button" type="button" onClick={onClose} disabled={installing}>{updateMessages.later}</button>}
            {pending && (
              <button className="command-button" type="button" onClick={onInstall} disabled={installing}>
                {updateMessages.install}
              </button>
            )}
          </div>
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
