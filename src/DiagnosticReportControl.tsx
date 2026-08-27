import {
  CheckCircle2, ChevronDown, ChevronUp, Copy, FileDown, LoaderCircle,
  Send, ShieldAlert, Stethoscope, Wrench, X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";

import {
  chatDiagnosticAssistant, chooseDiagnosticExportDestination, copyDiagnosticBundle, exportDiagnosticBundle,
  getDiagnosticReport, repairDiagnosticCustomProvider,
  type DiagnosticConversationMessage, type DiagnosticReport,
  type DiagnosticRepairPlanItem,
} from "./contracts/diagnostics";
import { listProviders, type ProviderSummary } from "./contracts/provider";

const quickPrompts = [
  "无法将供应商设置到 Codex", "返回 OpenAI 登录出错", "会话管理看不到会话",
  "Codex 启动或重启后仍无法使用", "配置被其他程序修改，应该怎么处理", "诊断报告里的问题我看不懂",
];
type ChatMessage = DiagnosticConversationMessage & { id: number; repairPlan?: DiagnosticRepairPlanItem[] };

export default function DiagnosticReportControl() {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [report, setReport] = useState<DiagnosticReport | null>(null);
  const [failed, setFailed] = useState(false);
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  useEffect(() => {
    if (!open) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => { document.body.style.overflow = previousOverflow; };
  }, [open]);
  const runDiagnosis = () => {
    if (loading) return;
    setOpen(true); setLoading(true); setReport(null); setFailed(false);
    void Promise.all([getDiagnosticReport(), listProviders().catch(() => [] as ProviderSummary[])])
      .then(([nextReport, nextProviders]) => { setReport(nextReport); setProviders(nextProviders); })
      .catch(() => setFailed(true)).finally(() => setLoading(false));
  };
  return <>
    <button className="secondary-button compact diagnostic-trigger" type="button" onClick={runDiagnosis} disabled={loading} title="查看本机诊断并与 AI 对话排查问题">
      {loading ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" /> : <Stethoscope className="button-icon is-teal" size={16} aria-hidden="true" />}帮帮我
    </button>
    {open && createPortal(<div className="dialog-backdrop diagnostic-report-backdrop"><section className="confirmation-dialog diagnostic-report-dialog" role="dialog" aria-modal="true" aria-labelledby="diagnostic-report-title">
      <header className="diagnostic-report-header"><div><h2 id="diagnostic-report-title">帮帮我</h2><p>AI 将结合脱敏诊断和你输入的问题协助排查；需要修改配置时，会先显示可回滚的操作计划，且不会直接执行任意命令。</p></div><button className="field-icon-button" type="button" onClick={() => setOpen(false)} aria-label="关闭帮帮我"><X size={17} aria-hidden="true" /></button></header>
      {loading && <div className="diagnostic-report-loading" role="status"><LoaderCircle className="is-spinning" size={20} aria-hidden="true" />正在检查当前用户 Codex 环境</div>}
      {!loading && failed && <div className="diagnostic-report-failure" role="alert"><ShieldAlert size={20} aria-hidden="true" /><div><strong>诊断失败</strong><p>无法读取完整的本机诊断信息，请重试。</p></div><button className="secondary-button" type="button" onClick={runDiagnosis}>重新检查</button></div>}
      {!loading && report && <DiagnosticWorkspace report={report} onReport={setReport} providers={providers} />}
    </section></div>, document.body)}
  </>;
}

function DiagnosticWorkspace({ report, onReport, providers }: { report: DiagnosticReport; onReport: (report: DiagnosticReport) => void; providers: ProviderSummary[] }) {
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [providerId, setProviderId] = useState(providers.find((provider) => provider.isCurrent)?.id ?? providers[0]?.id ?? "");
  const [draft, setDraft] = useState("");
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [sending, setSending] = useState(false);
  const [reportAction, setReportAction] = useState<"copy" | "export" | null>(null);
  const [exportFeedback, setExportFeedback] = useState<string | null>(null);
  const [exportFailed, setExportFailed] = useState(false);
  const [repairing, setRepairing] = useState(false);
  useEffect(() => { if (!providerId && providers[0]) setProviderId(providers.find((provider) => provider.isCurrent)?.id ?? providers[0].id); }, [providerId, providers]);
  const summary = useMemo(() => {
    const errors = report.findings.filter((finding) => finding.severity === "error").length;
    const warnings = report.findings.filter((finding) => finding.severity === "warning").length;
    if (errors > 0) return { tone: "error", text: `发现 ${errors} 个需要处理的问题${warnings ? `，另有 ${warnings} 个提醒` : ""}` };
    if (warnings > 0) return { tone: "warning", text: `发现 ${warnings} 个提醒，暂未发现阻断问题` };
    return { tone: "ok", text: "本机诊断未发现问题" };
  }, [report]);
  const sendMessage = (value: string) => {
    const message = value.trim();
    if (!message || !providerId || sending) return;
    const userMessage: ChatMessage = { id: Date.now(), role: "user", content: message };
    const history = messages.map(({ role, content }) => ({ role, content }));
    setMessages((current) => [...current, userMessage]); setDraft(""); setSending(true);
    void chatDiagnosticAssistant(providerId, message, history)
      .then((result) => setMessages((current) => [...current, { id: Date.now() + 1, role: "assistant", content: result.reply, repairPlan: result.repairPlan }]))
      .catch(() => setMessages((current) => [...current, { id: Date.now() + 1, role: "system", content: "AI 请求失败或供应商不可用。你仍可以复制或导出脱敏诊断结果。" }]))
      .finally(() => setSending(false));
  };
  const executeRepair = (previewId: string) => {
    if (repairing) return;
    setRepairing(true);
    void repairDiagnosticCustomProvider(previewId).then((execution) => {
      onReport(execution.report);
      setMessages((current) => [...current, { id: Date.now() + 2, role: "system", content: execution.status === "succeeded" ? "修复已完成，正在使用新的诊断结果。" : "修复未完成，原配置已保留或回滚。" }]);
    }).catch(() => setMessages((current) => [...current, { id: Date.now() + 2, role: "system", content: "修复状态无法确认，请查看诊断详情并复制或导出结果。" }])).finally(() => setRepairing(false));
  };
  const conversation = () => messages.map(({ role, content }) => ({ role, content }));
  const handleCopy = () => {
    if (reportAction) return;
    setReportAction("copy"); setExportFeedback(null); setExportFailed(false);
    void copyDiagnosticBundle(conversation())
      .then(() => setExportFeedback("已复制诊断信息"))
      .catch(() => { setExportFailed(true); setExportFeedback("复制失败，请重试。"); })
      .finally(() => setReportAction(null));
  };
  const handleExport = () => {
    if (reportAction) return;
    setReportAction("export"); setExportFeedback(null); setExportFailed(false);
    void chooseDiagnosticExportDestination()
      .then((destination) => destination
        ? exportDiagnosticBundle(destination, conversation()).then(() => true)
        : false)
      .then((exported) => { if (exported) setExportFeedback("已导出诊断信息"); })
      .catch(() => { setExportFailed(true); setExportFeedback("导出失败，请重新选择保存位置。"); }).finally(() => setReportAction(null));
  };
  return <div className="diagnostic-workspace">
    <div className={`diagnostic-summary is-${summary.tone}`} role="status">{summary.tone === "ok" ? <CheckCircle2 size={17} aria-hidden="true" /> : <ShieldAlert size={17} aria-hidden="true" />}<strong>{summary.text}</strong><button className="diagnostic-details-toggle" type="button" onClick={() => setDetailsOpen((value) => !value)} aria-expanded={detailsOpen}>{detailsOpen ? <ChevronUp size={15} aria-hidden="true" /> : <ChevronDown size={15} aria-hidden="true" />}{detailsOpen ? "收起详情" : "查看详情"}</button></div>
    <section className="diagnostic-chat" aria-label="和 AI 一起排查">
      <div className="diagnostic-chat-scroll">
        {detailsOpen && <DiagnosticDetails report={report} />}
        {messages.length === 0 && <div className="diagnostic-quick-prompts"><span>可以从这里开始</span>{quickPrompts.map((prompt) => <button key={prompt} type="button" onClick={() => sendMessage(prompt)} disabled={sending}>{prompt}</button>)}</div>}
        {messages.length > 0 && <div className="diagnostic-chat-messages" aria-live="polite">{messages.map((message) => <ChatBubble key={message.id} message={message} onRepair={executeRepair} repairing={repairing} />)}{sending && <div className="diagnostic-chat-bubble assistant is-pending"><LoaderCircle className="is-spinning" size={15} aria-hidden="true" />正在分析</div>}</div>}
        {providers.length === 0 && <p className="diagnostic-chat-empty">没有已验证供应商，暂时无法发起 AI 对话。你仍可以复制或导出本机诊断。</p>}
      </div>
      {providers.length > 0 && <div className="diagnostic-chat-composer"><textarea aria-label="向诊断助手提问" value={draft} onChange={(event) => setDraft(event.target.value)} placeholder="描述你遇到的现象，不要粘贴 API Key 或完整配置" rows={2} disabled={sending} onKeyDown={(event) => { if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) { event.preventDefault(); sendMessage(draft); } }} /><button className="primary-button" type="button" onClick={() => sendMessage(draft)} disabled={sending || !draft.trim() || !providerId}>{sending ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" /> : <Send size={16} aria-hidden="true" />}发送</button></div>}
    </section>
    <footer className="diagnostic-toolbar">
      {providers.length > 0 && <label>使用供应商<select aria-label="对话供应商" value={providerId} onChange={(event) => setProviderId(event.target.value)} disabled={sending}>{providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}{provider.isCurrent ? "（当前）" : ""}</option>)}</select></label>}
      <div className="diagnostic-export-actions">
        <button className="secondary-button diagnostic-export-button" type="button" onClick={handleCopy} disabled={reportAction !== null}>{reportAction === "copy" ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : <Copy size={15} aria-hidden="true" />}复制信息</button>
        <button className="secondary-button diagnostic-export-button" type="button" onClick={handleExport} disabled={reportAction !== null}>{reportAction === "export" ? <LoaderCircle className="is-spinning" size={15} aria-hidden="true" /> : <FileDown size={15} aria-hidden="true" />}导出信息</button>
      </div>
      {exportFeedback && <p role={exportFailed ? "alert" : "status"} className="diagnostic-export-feedback">{exportFeedback}</p>}
    </footer>
  </div>;
}

function DiagnosticDetails({ report }: { report: DiagnosticReport }) {
  return <div className="diagnostic-details"><dl className="diagnostic-facts"><div><dt>Codex 环境</dt><dd>{report.environment.codexHome}</dd></div><div><dt>配置</dt><dd>{configStatusLabels[report.environment.configStatus]}</dd></div><div><dt>当前 provider</dt><dd>{report.environment.activeProvider ?? "未设置"}</dd></div><div><dt>登录状态</dt><dd>{loginStatusLabels[report.authentication.loginStatus]}</dd></div><div><dt>桌面版</dt><dd>{consumerStatusLabels[report.consumers.desktop]}</dd></div><div><dt>Codex CLI</dt><dd>{consumerStatusLabels[report.consumers.cli]}</dd></div><div><dt>GPTEasy</dt><dd>{report.versions.gpteasy}</dd></div><div><dt>Codex CLI 版本</dt><dd>{report.versions.codexCli ?? "无法确认"}</dd></div></dl><section className="diagnostic-findings"><h4>诊断项</h4>{report.findings.length === 0 ? <p>未发现诊断项</p> : report.findings.map((finding) => <article key={`${finding.origin}:${finding.code}`}><strong>{finding.title}</strong><p>{finding.summary}</p></article>)}</section></div>;
}

function ChatBubble({ message, onRepair, repairing }: { message: ChatMessage; onRepair: (previewId: string) => void; repairing: boolean }) {
  const [confirming, setConfirming] = useState<string | null>(null);
  return <div className={`diagnostic-chat-bubble ${message.role}`}>{message.role !== "system" && <span className="diagnostic-chat-role">{message.role === "user" ? "你" : "AI"}</span>}<p>{message.content}</p>{message.repairPlan?.map((plan) => plan.previewId && <div className="diagnostic-action-card" key={plan.id}><div><strong>{plan.title}</strong><p>{plan.description}</p><small>将使用 GPTEasy 的确定性修复流程，执行前会创建备份。</small></div>{confirming === plan.previewId ? <div className="diagnostic-action-confirm"><span>确认按此计划修改当前用户 Codex 环境？</span><button className="primary-button" type="button" onClick={() => onRepair(plan.previewId!)} disabled={repairing}><Wrench size={15} aria-hidden="true" />{repairing ? "处理中" : "确认执行"}</button><button className="secondary-button" type="button" onClick={() => setConfirming(null)} disabled={repairing}>取消</button></div> : <button className="primary-button" type="button" onClick={() => setConfirming(plan.previewId!)} disabled={repairing}><Wrench size={15} aria-hidden="true" />查看并确认</button>}</div>)}</div>;
}

const configStatusLabels: Record<DiagnosticReport["environment"]["configStatus"], string> = { missing: "缺失", unreadable: "无法读取", encoding_error: "编码错误", toml_syntax_error: "TOML 语法错误", valid: "有效" };
const consumerStatusLabels: Record<DiagnosticReport["consumers"]["desktop"], string> = { running: "运行中", stopped: "已停止", unknown: "无法确认" };
const loginStatusLabels: Record<DiagnosticReport["authentication"]["loginStatus"], string> = { logged_in: "已认证", not_logged_in: "未认证", unavailable: "无法确认" };
