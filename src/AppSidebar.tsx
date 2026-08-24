import {
  Download,
  LoaderCircle,
  LogIn,
  MessageSquare,
  RefreshCw,
  Server,
  Settings,
  ScrollText,
  TriangleAlert,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";

import type { UpdateSnapshot } from "./contracts/update";

export interface UpdateSidebarState {
  snapshot: UpdateSnapshot;
  installing: boolean;
  onInstall: () => void;
  onOpen: () => void;
}

export interface OpenAiSidebarAction {
  busy: boolean;
  current: boolean;
  description: string;
  disabled: boolean;
  onSelect: () => void;
}

export default function AppSidebar({
  activeView = "providers",
  onOpenProviders,
  onOpenSessions,
  onOpenLogs,
  openAiAction,
  update,
  currentProviderName,
}: {
  activeView?: "providers" | "sessions" | "logs";
  onOpenProviders?: () => void;
  onOpenSessions?: () => void;
  onOpenLogs?: () => void;
  openAiAction?: OpenAiSidebarAction;
  update?: UpdateSidebarState;
  currentProviderName?: string | null;
}) {
  return (
    <aside className="sidebar" aria-label="应用导航">
      <Brand />
      <nav className="sidebar-nav" aria-label="主要菜单">
        <button
          className="nav-item"
          type="button"
          aria-current={activeView === "providers" ? "page" : undefined}
          onClick={onOpenProviders}
        >
          <Server size={18} aria-hidden="true" />
          供应商管理
        </button>
        <button
          className="nav-item"
          type="button"
          aria-current={activeView === "sessions" ? "page" : undefined}
          onClick={onOpenSessions}
        >
          <MessageSquare size={18} aria-hidden="true" />
          <span>会话管理</span>
        </button>
      </nav>
      <SidebarFooter currentProviderName={currentProviderName} openAiAction={openAiAction} update={update} onOpenLogs={onOpenLogs} />
    </aside>
  );
}

function SidebarFooter({
  currentProviderName,
  openAiAction,
  update,
  onOpenLogs,
}: {
  currentProviderName?: string | null;
  openAiAction?: OpenAiSidebarAction;
  update?: UpdateSidebarState;
  onOpenLogs?: () => void;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const settingsWrapRef = useRef<HTMLDivElement>(null);
  const providerLabel = currentProviderName || (openAiAction?.current ? "OpenAI 登录模式" : "未选择供应商");

  useEffect(() => {
    if (!settingsOpen) return;

    const handleDocumentClick = (event: MouseEvent) => {
      const target = event.target;
      if (target instanceof Node && !settingsWrapRef.current?.contains(target)) {
        setSettingsOpen(false);
      }
    };

    document.addEventListener("click", handleDocumentClick);
    return () => document.removeEventListener("click", handleDocumentClick);
  }, [settingsOpen]);

  return (
    <div className="sidebar-footer">
      <div className="sidebar-footer-row">
        <div className="sidebar-settings-wrap" ref={settingsWrapRef}>
          <button
            className="sidebar-settings-button"
            type="button"
            aria-label="设置"
            aria-expanded={settingsOpen}
            onClick={() => setSettingsOpen((value) => !value)}
          >
            <Settings size={17} aria-hidden="true" />
            <span className="sidebar-settings-label">设置</span>
          </button>
          {settingsOpen && (
            <div className="sidebar-settings-menu" role="menu">
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  if (!openAiAction) return;
                  setSettingsOpen(false);
                  openAiAction.onSelect();
                }}
                disabled={!openAiAction || openAiAction.disabled || openAiAction.current}
                title={openAiAction?.description ?? "正在读取 Codex 登录状态。"}
              >
                {openAiAction?.busy ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : <LogIn size={15} aria-hidden="true" />}
                返回 OpenAI 登录模式
              </button>
              {update && (
                <button type="button" role="menuitem" onClick={() => { setSettingsOpen(false); update.onOpen(); }}>
                  <RefreshCw size={15} aria-hidden="true" />
                  检查更新...
                </button>
              )}
              <button type="button" role="menuitem" onClick={() => { setSettingsOpen(false); onOpenLogs?.(); }}>
                <ScrollText size={15} aria-hidden="true" />
                问题日志
              </button>
            </div>
          )}
        </div>
        <span className="sidebar-provider-name" title={providerLabel}>{providerLabel}</span>
        {update && <SidebarUpdateIndicator update={update} />}
      </div>
    </div>
  );
}

type UpdateIndicatorKind = "checking" | "downloading" | "pending" | "installing" | "incomplete" | "failed";

function SidebarUpdateIndicator({ update }: { update: UpdateSidebarState }) {
  const { snapshot } = update;
  if (snapshot.state === "idle" || snapshot.state === "up_to_date") return null;

  let kind: UpdateIndicatorKind | null = null;
  if (update.installing) kind = "installing";
  else if (snapshot.state === "checking") kind = "checking";
  else if (snapshot.state === "downloading") kind = "downloading";
  else if (snapshot.state === "pending") kind = "pending";
  else if (snapshot.state === "incomplete") kind = "incomplete";
  else if (snapshot.state === "failed") kind = "failed";
  if (!kind) return null;

  const progress = snapshot.progressPercent;
  const label = kind === "installing"
    ? "正在启动更新"
    : kind === "checking"
      ? "正在检查更新"
    : kind === "downloading"
      ? progress == null ? "正在下载更新" : `正在下载更新 ${progress}%`
      : kind === "pending"
        ? "更新"
        : kind === "incomplete"
          ? "重试更新"
          : "更新失败";
  const onClick = kind === "pending" ? update.onInstall : update.onOpen;

  return (
    <div className={`sidebar-update-slot is-${kind}`} aria-live="polite">
      <button
        className={`sidebar-update-indicator is-${kind}${progress == null && kind === "downloading" ? " is-indeterminate" : ""}`}
        type="button"
        onClick={onClick}
        disabled={kind === "installing"}
        aria-label={label}
        aria-busy={kind === "installing" || undefined}
      >
        {kind === "checking" && <RefreshCw className="is-spinning" size={18} aria-hidden="true" />}
        {kind === "downloading" && progress != null && <span>{progress}%</span>}
        {kind === "downloading" && progress == null && (
          <>
            <LoaderCircle className="is-spinning" size={18} aria-hidden="true" />
            <span className="sidebar-update-motion-fallback">下载中</span>
          </>
        )}
        {kind === "pending" && (
          <>
            <Download size={18} aria-hidden="true" />
            <span className="sidebar-update-label">更新</span>
          </>
        )}
        {kind === "incomplete" && (
          <>
            <RefreshCw size={18} aria-hidden="true" />
            <span className="sidebar-update-label">重试更新</span>
          </>
        )}
        {kind === "failed" && (
          <>
            <TriangleAlert size={18} aria-hidden="true" />
            <span className="sidebar-update-label">更新失败</span>
          </>
        )}
        {kind === "installing" && (
          <>
            <LoaderCircle className="is-spinning" size={18} aria-hidden="true" />
            <span className="sidebar-update-motion-fallback">处理中</span>
          </>
        )}
      </button>
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
