export type AppPage = "providers" | "sessions" | "logs";

export const appPageTitles: Record<AppPage, string> = {
  providers: "供应商管理",
  sessions: "会话管理",
  logs: "问题日志",
};
