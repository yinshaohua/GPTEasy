import {
  LoaderCircle,
  LogIn,
  MessageSquare,
  PackageCheck,
  RefreshCw,
  Server,
} from "lucide-react";

import { providerMessages, updateMessages } from "./messages";
import type { UpdateSnapshot } from "./contracts/update";

export interface UpdateSidebarState {
  snapshot: UpdateSnapshot;
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
  openAiAction,
  update,
}: {
  activeView?: "providers" | "sessions";
  onOpenProviders?: () => void;
  onOpenSessions?: () => void;
  openAiAction?: OpenAiSidebarAction;
  update?: UpdateSidebarState;
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
          {openAiAction && (
            <button
              className="nav-item"
              type="button"
              onClick={openAiAction.onSelect}
              disabled={openAiAction.disabled}
              aria-pressed={openAiAction.current}
              aria-description={openAiAction.description}
            >
              {openAiAction.busy
                ? <LoaderCircle className="is-spinning" size={18} aria-hidden="true" />
                : <LogIn size={18} aria-hidden="true" />}
              <span>{providerMessages.switchToOpenAiLogin}</span>
            </button>
          )}
        </nav>
      )}
      {update && <UpdateStatus update={update} />}
    </aside>
  );
}

function UpdateStatus({ update }: { update: UpdateSidebarState }) {
  const { snapshot } = update;
  const label = snapshot.state === "pending"
    ? updateMessages.status.pending(snapshot.availableVersion)
    : updateMessages.status[snapshot.state];
  const Icon = snapshot.state === "pending" ? PackageCheck : RefreshCw;
  return (
    <div className="sidebar-update">
      <button className="sidebar-update-button" type="button" onClick={update.onOpen}>
        <Icon className={snapshot.state === "checking" || snapshot.state === "downloading" ? "is-spinning" : undefined} size={16} aria-hidden="true" />
        <span>{label}</span>
      </button>
      <button className="sidebar-version" type="button" onClick={update.onOpen}>
        <span>当前版本</span>
        <strong>v{snapshot.currentVersion}</strong>
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
