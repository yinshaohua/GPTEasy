import {
  CheckCircle2,
  FileJson,
  FileText,
  LoaderCircle,
  ShieldAlert,
  Stethoscope,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

import {
  chooseDiagnosticExportDestination,
  exportDiagnosticReport,
  getDiagnosticReport,
  type DiagnosticExportFormat,
  type DiagnosticConfigStatus,
  type DiagnosticConsumerStatus,
  type DiagnosticLoginStatus,
  type DiagnosticReport,
} from "./contracts/diagnostics";

const configStatusLabels: Record<DiagnosticConfigStatus, string> = {
  missing: "缺失",
  unreadable: "无法读取",
  encoding_error: "编码错误",
  toml_syntax_error: "TOML 语法错误",
  valid: "有效",
};

const consumerStatusLabels: Record<DiagnosticConsumerStatus, string> = {
  running: "运行中",
  stopped: "已停止",
  unknown: "无法确认",
};

const loginStatusLabels: Record<DiagnosticLoginStatus, string> = {
  logged_in: "已认证",
  not_logged_in: "未认证",
  unavailable: "无法确认",
};

export default function DiagnosticReportControl() {
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [report, setReport] = useState<DiagnosticReport | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    if (!open) return;
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previousOverflow;
    };
  }, [open]);

  const runDiagnosis = () => {
    if (loading) return;
    setOpen(true);
    setLoading(true);
    setReport(null);
    setFailed(false);
    void getDiagnosticReport()
      .then(setReport)
      .catch(() => setFailed(true))
      .finally(() => setLoading(false));
  };

  return (
    <>
      <button
        className="nav-item diagnostic-trigger"
        type="button"
        onClick={runDiagnosis}
        disabled={loading}
      >
        {loading
          ? <LoaderCircle className="is-spinning" size={18} aria-hidden="true" />
          : <Stethoscope size={18} aria-hidden="true" />}
        帮我排查
      </button>
      {open && createPortal((
        <div className="dialog-backdrop">
          <section
            className="confirmation-dialog diagnostic-report-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="diagnostic-report-title"
          >
            <header className="diagnostic-report-header">
              <div>
                <h2 id="diagnostic-report-title">本机诊断报告</h2>
                <p>当前用户 Codex 环境</p>
              </div>
              <button
                className="field-icon-button"
                type="button"
                onClick={() => setOpen(false)}
                aria-label="关闭诊断报告"
              >
                <X size={17} aria-hidden="true" />
              </button>
            </header>
            {loading && (
              <div className="diagnostic-report-loading" role="status">
                <LoaderCircle className="is-spinning" size={20} aria-hidden="true" />
                正在检查当前用户 Codex 环境
              </div>
            )}
            {!loading && failed && (
              <div className="diagnostic-report-failure" role="alert">
                <ShieldAlert size={20} aria-hidden="true" />
                <div>
                  <strong>诊断失败</strong>
                  <p>无法读取完整的本机诊断信息，请重试。</p>
                </div>
                <button className="secondary-button" type="button" onClick={runDiagnosis}>
                  重新检查
                </button>
              </div>
            )}
            {!loading && report && <DiagnosticReportResult report={report} />}
          </section>
        </div>
      ), document.body)}
    </>
  );
}

function DiagnosticReportResult({ report }: { report: DiagnosticReport }) {
  const noRepairableFindings = report.findings.every((finding) => !finding.repairable);
  const [exporting, setExporting] = useState<DiagnosticExportFormat | null>(null);
  const [exportFeedback, setExportFeedback] = useState<string | null>(null);
  const [exportFailed, setExportFailed] = useState(false);

  const handleExport = (format: DiagnosticExportFormat) => {
    if (exporting) return;
    setExporting(format);
    setExportFeedback(null);
    setExportFailed(false);
    void chooseDiagnosticExportDestination(format)
      .then((destination) => {
        if (!destination) return;
        return exportDiagnosticReport(format, destination).then(() => {
          setExportFeedback(format === "json" ? "JSON 已导出" : "Markdown 已导出");
        });
      })
      .catch(() => {
        setExportFailed(true);
        setExportFeedback("导出失败，请重新选择保存位置。");
      })
      .finally(() => setExporting(null));
  };

  return (
    <div className="diagnostic-report-result">
      <p className="diagnostic-report-success">
        <CheckCircle2 size={18} aria-hidden="true" />
        <strong>诊断完成</strong>
      </p>
      <dl className="diagnostic-facts">
        <div>
          <dt>Codex 环境</dt>
          <dd>{report.environment.codexHome}</dd>
        </div>
        <div>
          <dt>CODEX_HOME</dt>
          <dd>{report.environment.codexHomeOverrideStatus === "differs" ? "指向另一环境" : "使用当前用户默认环境"}</dd>
        </div>
        <div><dt>配置</dt><dd>{configStatusLabels[report.environment.configStatus]}</dd></div>
        <div><dt>当前 provider</dt><dd>{report.environment.activeProvider ?? "未设置"}</dd></div>
        <div>
          <dt>已声明 provider</dt>
          <dd>{report.environment.declaredProviders.join("、") || "无"}</dd>
        </div>
        <div><dt>认证</dt><dd>{loginStatusLabels[report.authentication.loginStatus]}</dd></div>
        <div>
          <dt>认证文件</dt>
          <dd>{({ missing: "缺失", present: "存在", unreadable: "无法读取" } as const)[report.authentication.authFileStatus]}</dd>
        </div>
        <div><dt>桌面版</dt><dd>{consumerStatusLabels[report.consumers.desktop]}</dd></div>
        <div><dt>Codex CLI</dt><dd>{consumerStatusLabels[report.consumers.cli]}</dd></div>
        <div><dt>GPTEasy 版本</dt><dd>{report.versions.gpteasy}</dd></div>
        <div><dt>Codex CLI 版本</dt><dd>{report.versions.codexCli ?? "无法确认"}</dd></div>
      </dl>
      <section className="diagnostic-findings" aria-labelledby="diagnostic-findings-title">
        <h3 id="diagnostic-findings-title">诊断项</h3>
        {report.findings.length === 0 && <p>未发现诊断项</p>}
        {report.findings.map((finding) => (
          <article key={`${finding.origin}:${finding.code}`}>
            <strong>{finding.title}</strong>
            <p>{finding.summary}</p>
          </article>
        ))}
      </section>
      {noRepairableFindings && (
        <p className="diagnostic-no-repair">没有可安全自动修复的项目</p>
      )}
      <div className="diagnostic-export-actions">
        <button
          className="secondary-button"
          type="button"
          onClick={() => handleExport("json")}
          disabled={exporting !== null}
        >
          {exporting === "json"
            ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
            : <FileJson size={16} aria-hidden="true" />}
          导出 JSON
        </button>
        <button
          className="secondary-button"
          type="button"
          onClick={() => handleExport("markdown")}
          disabled={exporting !== null}
        >
          {exporting === "markdown"
            ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
            : <FileText size={16} aria-hidden="true" />}
          导出 Markdown
        </button>
      </div>
      {exportFeedback && (
        <p role={exportFailed ? "alert" : "status"} className="diagnostic-export-feedback">
          {exportFeedback}
        </p>
      )}
    </div>
  );
}
