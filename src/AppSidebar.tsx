import {
  CircleCheck,
  Download,
  LoaderCircle,
  LogIn,
  MessageSquare,
  RefreshCw,
  Server,
  Settings,
} from "lucide-react";
import { useState } from "react";

import type { UpdateSnapshot } from "./contracts/update";

export interface UpdateSidebarState {
  snapshot: UpdateSnapshot;
  onOpen: () => void;
  onInstall?: () => void;
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
  openAiAction,
  update,
  currentProviderName,
}: {
  activeView?: "providers" | "sessions";
  onOpenProviders?: () => void;
  onOpenSessions?: () => void;
  openAiAction?: OpenAiSidebarAction;
  update?: UpdateSidebarState;
  currentProviderName?: string | null;
}) {
  return (
    <aside className="sidebar" aria-label="应用导航">
      <Brand />
      {(openAiAction || onOpenProviders || onOpenSessions) && (
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
      )}
      <SidebarFooter currentProviderName={currentProviderName} openAiAction={openAiAction} update={update} />
    </aside>
  );
}

function SidebarFooter({
  currentProviderName,
  openAiAction,
  update,
}: {
  currentProviderName?: string | null;
  openAiAction?: OpenAiSidebarAction;
  update?: UpdateSidebarState;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const snapshot = update?.snapshot;
  const hasUpdate = Boolean(snapshot && ["downloading", "pending", "incomplete"].includes(snapshot.state) && snapshot.availableVersion);
  const isDownloading = snapshot?.state === "downloading";
  const isReady = snapshot?.state === "pending" || snapshot?.state === "incomplete";
  const providerLabel = currentProviderName || (openAiAction?.current ? "OpenAI 登录模式" : "未选择供应商");

  return (
    <div className="sidebar-footer">
      <div className="sidebar-footer-row">
        <div className="sidebar-settings-wrap">
          <button
            className="sidebar-settings-button"
            type="button"
            aria-label="设置"
            aria-expanded={settingsOpen}
            onClick={() => setSettingsOpen((value) => !value)}
          >
            <Settings size={17} aria-hidden="true" />
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
            </div>
          )}
        </div>
        <span className="sidebar-provider-name" title={providerLabel}>{providerLabel}</span>
        {hasUpdate && update && (
          <button
            className={`sidebar-update-indicator${isReady ? " is-ready" : ""}${isDownloading ? " is-downloading" : ""}`}
            type="button"
            onClick={isReady && update.onInstall ? update.onInstall : update.onOpen}
            title={isReady ? "点击重启升级" : undefined}
            aria-label={isReady ? "点击重启升级" : `下载更新${snapshot?.progressPercent == null ? "" : ` ${snapshot.progressPercent}%`}`}
          >
            {isReady ? <CircleCheck size={18} aria-hidden="true" /> : <Download size={16} aria-hidden="true" />}
            {isDownloading && <span>{snapshot?.progressPercent == null ? "..." : `${snapshot.progressPercent}%`}</span>}
          </button>
        )}
      </div>
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
