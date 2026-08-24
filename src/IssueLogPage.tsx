import { Clipboard, Download, LoaderCircle, RefreshCw, Search } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

import {
  chooseIssueLogExportDestination,
  copyIssueLogs,
  exportIssueLogs,
  exportAllIssueLogs,
  listIssueLogs,
  type IssueLogFilter,
  type IssueLogLevel,
  type IssueLogRecord,
} from "./contracts/diagnostics";

const DAY_SECONDS = 24 * 60 * 60;

export default function IssueLogPage({ active = true }: { active?: boolean }) {
  const [days, setDays] = useState("7");
  const [level, setLevel] = useState<IssueLogLevel | "all">("all");
  const [query, setQuery] = useState("");
  const [records, setRecords] = useState<IssueLogRecord[]>([]);
  const [loading, setLoading] = useState(false);
  const [feedback, setFeedback] = useState("");

  const filter = useMemo<IssueLogFilter>(() => ({
    sinceEpochSeconds: Math.floor(Date.now() / 1000) - Number(days) * DAY_SECONDS,
    level: level === "all" ? null : level,
    query,
  }), [days, level, query]);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setRecords(await listIssueLogs(filter));
      setFeedback("");
    } catch {
      setFeedback("问题日志读取失败。");
    } finally {
      setLoading(false);
    }
  }, [filter]);

  useEffect(() => {
    if (active) void refresh();
  }, [active, refresh]);

  async function copy() {
    try {
      const count = await copyIssueLogs(filter);
      setFeedback(`已复制 ${count} 条日志。`);
    } catch {
      setFeedback("复制失败。");
    }
  }

  async function exportLogs() {
    const destination = await chooseIssueLogExportDestination();
    if (!destination) return;
    try {
      const count = await exportIssueLogs(filter, destination);
      setFeedback(`已导出 ${count} 条日志。`);
    } catch {
      setFeedback("导出失败。");
    }
  }

  async function exportAllLogs() {
    const destination = await chooseIssueLogExportDestination();
    if (!destination) return;
    try {
      const count = await exportAllIssueLogs(destination);
      setFeedback(`已导出全部 ${count} 条日志。`);
    } catch {
      setFeedback("导出失败。");
    }
  }

  return (
    <main className="main-content issue-log-page">
      <header className="page-header">
        <div>
          <p className="eyebrow">诊断</p>
          <h1>问题日志</h1>
        </div>
        <div className="page-header-actions">
          <button className="secondary-button compact" type="button" onClick={() => void refresh()} disabled={loading} title="重新读取日志">
            {loading ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" /> : <RefreshCw size={16} aria-hidden="true" />}
            刷新
          </button>
          <button className="secondary-button compact" type="button" onClick={() => void copy()} disabled={loading} title="复制当前筛选结果">
            <Clipboard size={16} aria-hidden="true" />复制
          </button>
          <button className="command-button compact" type="button" onClick={() => void exportLogs()} disabled={loading} title="导出当前筛选结果">
            <Download size={16} aria-hidden="true" />导出
          </button>
          <button className="secondary-button compact" type="button" onClick={() => void exportAllLogs()} disabled={loading} title="导出全部日志">
            <Download size={16} aria-hidden="true" />导出全部
          </button>
        </div>
      </header>
      <section className="issue-log-toolbar" aria-label="日志筛选">
        <label>时间范围<select value={days} onChange={(event) => setDays(event.target.value)}><option value="1">近 1 天</option><option value="7">近 7 天</option><option value="30">近 30 天</option><option value="3650">全部</option></select></label>
        <label>级别<select value={level} onChange={(event) => setLevel(event.target.value as IssueLogLevel | "all")}><option value="all">全部</option><option value="error">错误</option><option value="warn">警告</option><option value="info">信息</option></select></label>
        <label className="issue-log-search">关键词<div className="input-with-icon"><Search size={16} aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="事件或内容" /></div></label>
      </section>
      {feedback && <p className="inline-feedback" role="status">{feedback}</p>}
      <section className="issue-log-list" aria-live="polite">
        {loading && <p className="pane-note">正在读取问题日志...</p>}
        {!loading && records.length === 0 && <p className="pane-note">当前筛选范围没有日志。</p>}
        {!loading && records.map((record, index) => (
          <article className={`issue-log-entry is-${record.level}`} key={`${record.timestampEpochSeconds}-${index}`}>
            <div className="issue-log-entry-meta"><time dateTime={new Date(record.timestampEpochSeconds * 1000).toISOString()}>{new Date(record.timestampEpochSeconds * 1000).toLocaleString("zh-CN")}</time><span>{record.level === "error" ? "错误" : record.level === "warn" ? "警告" : "信息"}</span><code>{record.event}</code></div>
            <strong>{record.message}</strong>
            {record.details && <pre>{record.details}</pre>}
          </article>
        ))}
      </section>
    </main>
  );
}
