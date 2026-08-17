import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import SessionPage from "./SessionPage";

const sessionContract = vi.hoisted(() => ({
  archiveSessions: vi.fn(),
  cancelSessionRequest: vi.fn(),
  chooseSessionExportDestination: vi.fn(),
  deleteSession: vi.fn(),
  enterSessionManagement: vi.fn(),
  exportSessionMarkdown: vi.fn(),
  leaveSessionManagement: vi.fn(),
  listSessions: vi.fn(),
  readSession: vi.fn(),
  unarchiveSessions: vi.fn(),
}));

vi.mock("./contracts/session", () => sessionContract);

const firstSession = {
  id: "thread-1",
  title: "登录修复",
  preview: "修复登录流程",
  project: "C:\\src\\demo",
  modelProvider: "history-provider",
  source: "Codex CLI",
  createdAt: 1_786_900_000,
  updatedAt: 1_786_900_300,
};

const secondSession = {
  ...firstSession,
  id: "thread-2",
  title: "发布检查",
  preview: "检查发布候选",
  project: "C:\\src\\release",
  modelProvider: "openai",
};

describe("会话管理页面", () => {
  beforeEach(() => {
    for (const mock of Object.values(sessionContract)) mock.mockReset();
    sessionContract.enterSessionManagement.mockResolvedValue({
      status: "available",
      messageId: "session.available",
      codexVersion: "codex-cli 0.147.0",
      mutation: {
        status: "allowed",
        messageId: "session.mutations_allowed",
      },
    });
    sessionContract.archiveSessions.mockResolvedValue([]);
    sessionContract.cancelSessionRequest.mockResolvedValue(true);
    sessionContract.unarchiveSessions.mockResolvedValue([]);
    sessionContract.deleteSession.mockResolvedValue({
      sessionId: "thread-1",
      status: "succeeded",
      actualState: "deleted",
      messageId: "session.deleted",
    });
    sessionContract.leaveSessionManagement.mockResolvedValue(undefined);
    sessionContract.exportSessionMarkdown.mockResolvedValue(undefined);
    sessionContract.chooseSessionExportDestination.mockResolvedValue(null);
    sessionContract.readSession.mockResolvedValue({
      ...firstSession,
      entries: [
        { id: "user", kind: "user", label: "用户", content: "请修复登录", output: null },
        { id: "tool", kind: "tool", label: "命令", content: "npm test", output: "all passed" },
        { id: "assistant", kind: "assistant", label: "助手", content: "登录流程已修复。", output: null },
      ],
    });
    sessionContract.listSessions.mockImplementation((query: { cursor: string | null }) => {
      if (query.cursor === "cursor-2") {
        return Promise.resolve({ sessions: [firstSession, secondSession], nextCursor: null });
      }
      return Promise.resolve({ sessions: [firstSession], nextCursor: "cursor-2" });
    });
  });

  afterEach(() => cleanup());

  it("分页去重，查询变化重置 cursor，查看详情后保留列表范围", async () => {
    render(<SessionPage onOpenProviders={() => undefined} />);

    expect(await screen.findByRole("heading", { name: "会话管理" })).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: "会话" })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: "已归档" })).toHaveAttribute("aria-selected", "false");
    expect(await screen.findByRole("button", { name: "打开会话：登录修复" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "加载更多" }));
    expect(await screen.findByRole("button", { name: "打开会话：发布检查" })).toBeInTheDocument();
    expect(screen.getAllByRole("row")).toHaveLength(3);

    fireEvent.click(screen.getByRole("button", { name: "打开会话：登录修复" }));
    expect(await screen.findByRole("heading", { name: "登录修复" })).toBeInTheDocument();
    expect(screen.getByText("请修复登录")).toBeInTheDocument();
    expect(screen.getByText("登录流程已修复。")).toBeInTheDocument();
    const activity = screen.getByText("命令").closest("details");
    expect(activity).not.toHaveAttribute("open");

    fireEvent.click(screen.getByRole("button", { name: "返回会话列表" }));
    expect(screen.getByRole("button", { name: "打开会话：发布检查" })).toBeInTheDocument();

    fireEvent.change(screen.getByRole("searchbox", { name: "搜索标题或预览" }), {
      target: { value: "发布" },
    });
    await waitFor(() => {
      const latest = sessionContract.listSessions.mock.calls.at(-1)?.[0];
      expect(latest).toMatchObject({ searchTerm: "发布", cursor: null });
    });
  });

  it("页面暂时离开或切换页签后复用已读取的列表缓存", async () => {
    const { rerender } = render(<SessionPage active onOpenProviders={() => undefined} />);

    expect(await screen.findByRole("button", { name: "打开会话：登录修复" })).toBeInTheDocument();
    expect(sessionContract.listSessions).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByRole("tab", { name: "已归档" }));
    await waitFor(() => expect(sessionContract.listSessions).toHaveBeenCalledTimes(2));
    expect(await screen.findByRole("button", { name: "打开会话：登录修复" })).toBeInTheDocument();

    fireEvent.click(screen.getByRole("tab", { name: "会话" }));
    await waitFor(() => expect(screen.getByRole("button", { name: "打开会话：登录修复" })).toBeInTheDocument());
    expect(sessionContract.listSessions).toHaveBeenCalledTimes(2);

    rerender(<SessionPage active={false} onOpenProviders={() => undefined} />);
    await waitFor(() => expect(sessionContract.leaveSessionManagement).toHaveBeenCalledTimes(1));
    rerender(<SessionPage active onOpenProviders={() => undefined} />);
    await waitFor(() => expect(sessionContract.enterSessionManagement).toHaveBeenCalledTimes(2));
    expect(sessionContract.listSessions).toHaveBeenCalledTimes(2);
  });

  it("筛选变化会取消仍在途的旧列表请求", async () => {
    let resolveFirst: ((value: { sessions: typeof firstSession[]; nextCursor: null }) => void) | undefined;
    sessionContract.listSessions
      .mockImplementationOnce(() => new Promise((resolve) => { resolveFirst = resolve; }))
      .mockResolvedValue({ sessions: [firstSession], nextCursor: null });
    render(<SessionPage onOpenProviders={() => undefined} />);

    await waitFor(() => expect(sessionContract.listSessions).toHaveBeenCalled());
    fireEvent.change(screen.getByRole("searchbox", { name: "搜索标题或预览" }), {
      target: { value: "发布" },
    });
    await waitFor(() => expect(sessionContract.cancelSessionRequest).toHaveBeenCalledWith(expect.any(String)));
    resolveFirst?.({ sessions: [firstSession], nextCursor: null });
  });

  it("导出取消不写文件，选择目标后提交当前结构化详情快照", async () => {
    render(<SessionPage onOpenProviders={() => undefined} />);
    fireEvent.click(await screen.findByRole("button", { name: "打开会话：登录修复" }));
    await screen.findByRole("heading", { name: "登录修复" });

    fireEvent.click(screen.getByRole("button", { name: "导出 Markdown" }));
    await waitFor(() => expect(sessionContract.chooseSessionExportDestination).toHaveBeenCalled());
    expect(sessionContract.exportSessionMarkdown).not.toHaveBeenCalled();

    sessionContract.chooseSessionExportDestination.mockResolvedValue("C:\\exports\\登录修复.md");
    fireEvent.click(screen.getByRole("button", { name: "导出 Markdown" }));
    await waitFor(() => expect(sessionContract.exportSessionMarkdown).toHaveBeenCalledWith(
      expect.objectContaining({
        id: "thread-1",
        entries: expect.arrayContaining([
          expect.objectContaining({ id: "user", content: "请修复登录" }),
        ]),
      }),
      "C:\\exports\\登录修复.md",
    ));
  });

  it("协议不可用与恢复失败显示不同状态和动作", async () => {
    sessionContract.enterSessionManagement.mockResolvedValue({
      status: "incompatible",
      messageId: "session.incompatible",
      codexVersion: "codex-cli 0.120.0",
    });
    const { unmount } = render(<SessionPage onOpenProviders={() => undefined} />);
    expect(await screen.findByRole("heading", { name: "需要升级 Codex" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重新检查" })).toBeInTheDocument();
    unmount();

    sessionContract.enterSessionManagement.mockResolvedValue({
      status: "recovery_failed",
      messageId: "session.recovery_failed",
      codexVersion: null,
    });
    render(<SessionPage onOpenProviders={() => undefined} />);
    expect(await screen.findByRole("heading", { name: "会话服务恢复失败" })).toBeInTheDocument();
    expect(within(screen.getByRole("alert")).getByRole("button", { name: "重新检查" })).toBeInTheDocument();
  });

  it("返回列表后忽略迟到的详情响应", async () => {
    let resolveDetail: ((detail: unknown) => void) | undefined;
    sessionContract.readSession.mockReturnValue(new Promise((resolve) => {
      resolveDetail = resolve;
    }));
    render(<SessionPage onOpenProviders={() => undefined} />);

    fireEvent.click(await screen.findByRole("button", { name: "打开会话：登录修复" }));
    expect(screen.getByRole("button", { name: "返回会话列表" })).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "返回会话列表" }));
    expect(screen.getByRole("heading", { name: "会话管理" })).toBeInTheDocument();

    resolveDetail?.({ ...firstSession, entries: [] });
    await waitFor(() => {
      expect(screen.getByRole("heading", { name: "会话管理" })).toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "返回会话列表" })).not.toBeInTheDocument();
    });
  });

  it("项目和会话供应商筛选接受首批结果之外的值", async () => {
    render(<SessionPage onOpenProviders={() => undefined} />);
    await screen.findByRole("button", { name: "打开会话：登录修复" });

    fireEvent.change(screen.getByRole("combobox", { name: "项目筛选" }), {
      target: { value: "C:\\src\\not-loaded" },
    });
    fireEvent.change(screen.getByRole("combobox", { name: "会话供应商筛选" }), {
      target: { value: "not-loaded-provider" },
    });

    await waitFor(() => {
      expect(sessionContract.listSessions.mock.calls.at(-1)?.[0]).toMatchObject({
        project: "C:\\src\\not-loaded",
        modelProvider: "not-loaded-provider",
        cursor: null,
      });
    });
  });

  it("消费者运行时保持列表和详情可读，但禁用全部会话修改", async () => {
    sessionContract.enterSessionManagement.mockResolvedValue({
      status: "available",
      messageId: "session.available",
      codexVersion: "codex-cli 0.147.0",
      mutation: {
        status: "consumers_running",
        messageId: "session.consumers_running",
      },
    });

    render(<SessionPage onOpenProviders={() => undefined} />);

    expect(await screen.findByRole("button", { name: "打开会话：登录修复" })).toBeInTheDocument();
    expect(screen.getByRole("status", { name: "会话修改已禁用" })).toHaveTextContent(
      "检测到外部 Codex 消费者正在运行",
    );
    expect(screen.getByRole("checkbox", { name: "选择会话：登录修复" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "归档会话：登录修复" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "永久删除会话：登录修复" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "打开会话：登录修复" }));
    expect(await screen.findByRole("heading", { name: "登录修复" })).toBeInTheDocument();
    expect(screen.getByText("请修复登录")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "归档会话" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "永久删除会话" })).toBeDisabled();
  });

  it("批量归档逐项保留部分成功，失败项可单独重试且不自动重发", async () => {
    sessionContract.listSessions.mockResolvedValue({
      sessions: [firstSession, secondSession],
      nextCursor: null,
    });
    sessionContract.archiveSessions.mockResolvedValue([
      {
        sessionId: "thread-1",
        status: "succeeded",
        actualState: "archived",
        messageId: "session.archived",
      },
      {
        sessionId: "thread-2",
        status: "failed",
        actualState: "active",
        messageId: "session.request_failed",
      },
    ]);
    render(<SessionPage onOpenProviders={() => undefined} />);

    fireEvent.click(await screen.findByRole("checkbox", { name: "选择会话：登录修复" }));
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话：发布检查" }));
    expect(screen.queryByRole("button", { name: /永久删除所选/ })).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "归档所选会话" }));

    await waitFor(() => expect(sessionContract.archiveSessions).toHaveBeenCalledWith([
      "thread-1",
      "thread-2",
    ]));
    expect(sessionContract.archiveSessions).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("button", { name: "打开会话：登录修复" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "打开会话：发布检查" })).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("已归档 1 个，1 个未完成");
    const outcomeList = screen.getByRole("list", { name: "逐项操作结果" });
    expect(within(outcomeList).getByText("登录修复：已归档")).toBeInTheDocument();
    expect(within(outcomeList).getByText("发布检查：仍在会话")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "重试归档：发布检查" })).toBeInTheDocument();
  });

  it("永久删除仅确认单项，展示影响说明并允许可选导出", async () => {
    render(<SessionPage onOpenProviders={() => undefined} />);
    fireEvent.click(await screen.findByRole("button", { name: "打开会话：登录修复" }));
    await screen.findByRole("heading", { name: "登录修复" });

    fireEvent.click(screen.getByRole("button", { name: "永久删除会话" }));
    const initialDialog = screen.getByRole("dialog", { name: "永久删除会话" });
    const cancelButton = within(initialDialog).getByRole("button", { name: "取消" });
    expect(cancelButton).toHaveFocus();
    fireEvent.keyDown(initialDialog, { key: "Escape" });
    expect(screen.queryByRole("dialog", { name: "永久删除会话" })).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "返回会话列表" }));
    fireEvent.click(screen.getByRole("button", { name: "打开会话：登录修复" }));
    await screen.findByRole("heading", { name: "登录修复" });
    fireEvent.click(screen.getByRole("button", { name: "永久删除会话" }));
    const reopenedDialog = screen.getByRole("dialog", { name: "永久删除会话" });
    expect(within(reopenedDialog).getByRole("button", { name: "取消" })).toHaveFocus();
    fireEvent.keyDown(reopenedDialog, { key: "Tab", shiftKey: true });
    expect(within(reopenedDialog).getByRole("button", { name: "永久删除会话" })).toHaveFocus();

    const dialog = reopenedDialog;
    expect(within(dialog).getByText("登录修复")).toBeInTheDocument();
    expect(within(dialog).getByText("C:\\src\\demo")).toBeInTheDocument();
    expect(within(dialog).getByText(/不可撤销/)).toBeInTheDocument();
    expect(within(dialog).getByText(/派生会话的完整范围无法完全列出/)).toBeInTheDocument();
    expect(within(dialog).queryByRole("textbox")).not.toBeInTheDocument();

    fireEvent.click(within(dialog).getByRole("button", { name: "先导出 Markdown" }));
    await waitFor(() => expect(sessionContract.chooseSessionExportDestination).toHaveBeenCalled());
    expect(sessionContract.deleteSession).not.toHaveBeenCalled();

    fireEvent.click(within(dialog).getByRole("button", { name: "永久删除会话" }));
    await waitFor(() => expect(sessionContract.deleteSession).toHaveBeenCalledWith("thread-1"));
    expect(screen.queryByRole("dialog", { name: "永久删除会话" })).not.toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "会话管理" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "打开会话：登录修复" })).not.toBeInTheDocument();
  });

  it("已归档页支持多选取消归档", async () => {
    sessionContract.unarchiveSessions.mockResolvedValue([
      {
        sessionId: "thread-1",
        status: "succeeded",
        actualState: "active",
        messageId: "session.unarchived",
      },
    ]);
    render(<SessionPage onOpenProviders={() => undefined} />);
    fireEvent.click(await screen.findByRole("tab", { name: "已归档" }));
    await screen.findByRole("button", { name: "打开会话：登录修复" });
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话：登录修复" }));
    fireEvent.click(screen.getByRole("button", { name: "取消归档所选会话" }));

    await waitFor(() => expect(sessionContract.unarchiveSessions).toHaveBeenCalledWith(["thread-1"]));
    expect(screen.queryByRole("button", { name: "打开会话：登录修复" })).not.toBeInTheDocument();
  });

  it("删除确认显示当前已加载的派生会话并同时保留不完整范围提示", async () => {
    const derivedSession = { ...secondSession, forkedFromId: "thread-1" };
    sessionContract.listSessions.mockResolvedValue({
      sessions: [firstSession, derivedSession],
      nextCursor: null,
    });
    render(<SessionPage onOpenProviders={() => undefined} />);
    fireEvent.click(await screen.findByRole("button", { name: "打开会话：登录修复" }));
    await screen.findByRole("heading", { name: "登录修复" });
    fireEvent.click(screen.getByRole("button", { name: "永久删除会话" }));

    const dialog = screen.getByRole("dialog", { name: "永久删除会话" });
    expect(within(dialog).getByText("当前已识别 1 个派生会话也可能被删除。")).toBeInTheDocument();
    expect(within(dialog).getByText("发布检查")).toBeInTheDocument();
    expect(within(dialog).getByText(/完整范围无法完全列出/)).toBeInTheDocument();
  });
});
