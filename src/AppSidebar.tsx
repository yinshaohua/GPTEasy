import {
  LoaderCircle,
  LogIn,
  MessageSquare,
  Server,
} from "lucide-react";

import { providerMessages } from "./messages";

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
}: {
  activeView?: "providers" | "sessions";
  onOpenProviders?: () => void;
  onOpenSessions?: () => void;
  openAiAction?: OpenAiSidebarAction;
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
