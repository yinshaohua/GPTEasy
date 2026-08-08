import { useEffect, useRef, useState } from "react";
import {
  Check,
  Circle,
  Copy,
  Eye,
  EyeOff,
  KeyRound,
  LoaderCircle,
  Plus,
  Server,
  ShieldCheck,
  X,
} from "lucide-react";

import {
  asProviderFailure,
  cancelProviderRequest,
  discoverProviderModels,
  discardProviderValidation,
  listProviders,
  saveVerifiedProvider,
  validateProvider,
  type ProviderFailure,
  type ProviderSummary,
  type ProviderValidationReceipt,
} from "./contracts/provider";
import { providerFailureMessages } from "./messages";

type Operation = "idle" | "discovering" | "validating" | "verified" | "saving";

export default function ProviderPage() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [listState, setListState] = useState<"loading" | "ready" | "error">("loading");
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [defaultModel, setDefaultModel] = useState("");
  const [operation, setOperation] = useState<Operation>("idle");
  const [failure, setFailure] = useState<ProviderFailure | null>(null);
  const [receipt, setReceipt] = useState<ProviderValidationReceipt | null>(null);
  const [copyStatus, setCopyStatus] = useState("");
  const activeRequest = useRef<string | null>(null);
  const receiptRef = useRef<string | null>(null);

  useEffect(() => {
    let mounted = true;
    void listProviders()
      .then((items) => {
        if (mounted) {
          setProviders(items);
          setListState("ready");
        }
      })
      .catch(() => {
        if (mounted) setListState("error");
      });
    return () => {
      mounted = false;
      if (activeRequest.current) void cancelProviderRequest(activeRequest.current);
      if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    };
  }, []);

  function clearReceipt() {
    if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    receiptRef.current = null;
    setReceipt(null);
    setOperation("idle");
  }

  function changeConnection(field: "baseUrl" | "apiKey", value: string) {
    clearReceipt();
    setModels([]);
    setDefaultModel("");
    setFailure(null);
    if (field === "baseUrl") setBaseUrl(value);
    else setApiKey(value);
  }

  function changeModel(value: string) {
    clearReceipt();
    setDefaultModel(value);
    setFailure(null);
  }

  async function discoverModels() {
    clearReceipt();
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("discovering");
    setFailure(null);
    try {
      const result = await discoverProviderModels(requestId, baseUrl, apiKey);
      setBaseUrl(result.normalizedBaseUrl);
      setModels(result.models);
      setDefaultModel("");
      setOperation("idle");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setModels([]);
      setDefaultModel("");
      setOperation("idle");
    } finally {
      activeRequest.current = null;
    }
  }

  async function runValidation() {
    clearReceipt();
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("validating");
    setFailure(null);
    try {
      const result = await validateProvider(requestId, baseUrl, apiKey, defaultModel);
      receiptRef.current = result.validationId;
      setReceipt(result);
      setOperation("verified");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation("idle");
    } finally {
      activeRequest.current = null;
    }
  }

  async function cancelCurrentRequest() {
    if (activeRequest.current) await cancelProviderRequest(activeRequest.current);
  }

  async function saveProvider() {
    if (!receipt) return;
    setOperation("saving");
    setFailure(null);
    try {
      const saved = await saveVerifiedProvider(receipt.validationId, name);
      receiptRef.current = null;
      setProviders((current) =>
        [...current, saved].sort((left, right) => left.name.localeCompare(right.name, "zh-CN")),
      );
      resetEditor();
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation("verified");
    }
  }

  function resetEditor() {
    setName("");
    setBaseUrl("");
    setApiKey("");
    setShowKey(false);
    setModels([]);
    setDefaultModel("");
    setReceipt(null);
    setFailure(null);
    setOperation("idle");
  }

  async function copyApiKey() {
    try {
      await navigator.clipboard.writeText(apiKey);
      setCopyStatus("已复制");
    } catch {
      setCopyStatus("复制失败");
    }
  }

  const busy = operation === "discovering" || operation === "validating" || operation === "saving";
  const canDiscover = baseUrl.trim().length > 0 && apiKey.length > 0 && !busy;
  const canValidate = models.length > 0 && defaultModel.length > 0 && !busy;
  const canSave = operation === "verified" && name.trim().length > 0;

  return (
    <>
      <header className="page-header">
        <div>
          <h1>供应商</h1>
          <p>创建并保存通过完整验证的供应商</p>
        </div>
        <button className="command-button compact" type="button" onClick={resetEditor} disabled={busy}>
          <Plus size={17} aria-hidden="true" />
          新建供应商
        </button>
      </header>

      <div className="provider-workspace">
        <section className="provider-list-pane" aria-labelledby="provider-list-heading">
          <div className="pane-heading">
            <h2 id="provider-list-heading">已验证供应商</h2>
            <span>{providers.length}</span>
          </div>
          {listState === "loading" && <p className="pane-note">正在读取供应商目录</p>}
          {listState === "error" && <p className="inline-error">无法读取供应商目录。</p>}
          {listState === "ready" && providers.length === 0 && (
            <div className="empty-list">
              <Server size={22} aria-hidden="true" />
              <p>尚无已验证供应商</p>
            </div>
          )}
          <div className="provider-list">
            {providers.map((provider) => (
              <div className="provider-list-row" key={provider.id}>
                <strong>{provider.name}</strong>
                <span title={provider.defaultModel}>{provider.defaultModel}</span>
                <time dateTime={new Date(provider.verifiedAtEpochSeconds * 1000).toISOString()}>
                  {formatVerificationTime(provider.verifiedAtEpochSeconds)}
                </time>
              </div>
            ))}
          </div>
        </section>

        <section className="provider-editor" aria-labelledby="provider-editor-heading">
          <div className="pane-heading editor-heading">
            <div>
              <h2 id="provider-editor-heading">新建供应商</h2>
              <span>验证成功后再保存</span>
            </div>
          </div>

          <div className="field-grid">
            <label className="form-field full-width">
              <span>供应商名称</span>
              <input value={name} onChange={(event) => setName(event.target.value)} disabled={operation === "saving"} />
            </label>
            <label className="form-field full-width">
              <span>服务地址</span>
              <input
                type="url"
                value={baseUrl}
                onChange={(event) => changeConnection("baseUrl", event.target.value)}
                placeholder="https://provider.example/v1"
                disabled={busy}
              />
            </label>
            <div className="form-field full-width">
              <label htmlFor="provider-api-key">API Key</label>
              <div className="secret-input">
                <input
                  id="provider-api-key"
                  type={showKey ? "text" : "password"}
                  value={apiKey}
                  onChange={(event) => changeConnection("apiKey", event.target.value)}
                  autoComplete="off"
                  disabled={busy}
                />
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => setShowKey((current) => !current)}
                  aria-label={showKey ? "隐藏 API Key" : "显示 API Key"}
                  title={showKey ? "隐藏 API Key" : "显示 API Key"}
                  disabled={!apiKey}
                >
                  {showKey ? <EyeOff size={17} /> : <Eye size={17} />}
                </button>
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => void copyApiKey()}
                  aria-label="复制 API Key"
                  title="复制 API Key"
                  disabled={!apiKey}
                >
                  <Copy size={17} />
                </button>
              </div>
              <span className="sr-status" role="status">{copyStatus}</span>
            </div>
          </div>

          <div className="model-row">
            <label className="form-field model-select">
              <span>默认模型</span>
              <select
                value={defaultModel}
                onChange={(event) => changeModel(event.target.value)}
                disabled={models.length === 0 || busy}
              >
                <option value="">请选择模型</option>
                {models.map((model) => (
                  <option key={model} value={model}>{model}</option>
                ))}
              </select>
            </label>
            <button
              className="secondary-button"
              type="button"
              onClick={() => void discoverModels()}
              disabled={!canDiscover}
            >
              {operation === "discovering" ? (
                <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
              ) : (
                <Server size={17} aria-hidden="true" />
              )}
              获取模型
            </button>
          </div>

          <ValidationStatus modelsReady={models.length > 0} operation={operation} failure={failure} />

          <div className="editor-actions">
            {operation === "discovering" || operation === "validating" ? (
              <button className="secondary-button" type="button" onClick={() => void cancelCurrentRequest()}>
                <X size={17} aria-hidden="true" />
                取消请求
              </button>
            ) : (
              <button
                className="secondary-button"
                type="button"
                onClick={() => void runValidation()}
                disabled={!canValidate}
              >
                <ShieldCheck size={17} aria-hidden="true" />
                验证供应商
              </button>
            )}
            <button
              className="command-button"
              type="button"
              onClick={() => void saveProvider()}
              disabled={!canSave}
            >
              {operation === "saving" ? (
                <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
              ) : (
                <Check size={17} aria-hidden="true" />
              )}
              保存
            </button>
          </div>
        </section>
      </div>
    </>
  );
}

function ValidationStatus({
  modelsReady,
  operation,
  failure,
}: {
  modelsReady: boolean;
  operation: Operation;
  failure: ProviderFailure | null;
}) {
  const verified = operation === "verified" || operation === "saving";
  return (
    <div className="validation-panel" aria-live="polite">
      <div className="validation-title">
        <KeyRound size={18} aria-hidden="true" />
        <strong>{verified ? "完整验证已通过" : "供应商验证"}</strong>
      </div>
      <ol className="validation-steps">
        <ValidationStep complete={modelsReady} active={operation === "discovering"} label="模型确认" />
        <ValidationStep complete={verified} active={operation === "validating"} label="Responses 流式响应" />
        <ValidationStep complete={verified} active={operation === "validating"} label="工具调用闭环" />
      </ol>
      {failure && (
        <div className="validation-error" role="alert">
          <p>{providerFailureMessages[failure.messageId] ?? "验证未完成，请检查输入后重试。"}</p>
          <details>
            <summary>技术详情</summary>
            <code>{failure.category}</code>
          </details>
        </div>
      )}
    </div>
  );
}

function ValidationStep({ complete, active, label }: { complete: boolean; active: boolean; label: string }) {
  return (
    <li className={complete ? "is-complete" : active ? "is-active" : undefined}>
      {complete ? (
        <Check size={15} aria-hidden="true" />
      ) : active ? (
        <LoaderCircle className="is-spinning" size={15} aria-hidden="true" />
      ) : (
        <Circle size={15} aria-hidden="true" />
      )}
      {label}
    </li>
  );
}

let requestSequence = 0;

function createRequestId(): string {
  requestSequence += 1;
  return `provider-request-${Date.now()}-${requestSequence}`;
}

function formatVerificationTime(epochSeconds: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  }).format(new Date(epochSeconds * 1000));
}
