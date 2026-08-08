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
  onProviderValidationProgress,
  saveVerifiedProvider,
  validateProvider,
  type ProviderFailure,
  type ProviderSummary,
  type ProviderValidationReceipt,
  type ProviderValidationStage,
} from "./contracts/provider";
import { providerFailureMessages, providerMessages } from "./messages";

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
  const [validationStage, setValidationStage] = useState<ProviderValidationStage | "idle" | "complete">("idle");
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

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onProviderValidationProgress((progress) => {
      if (progress.requestId === activeRequest.current) setValidationStage(progress.stage);
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else unlisten = stopListening;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  function clearReceipt() {
    if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    receiptRef.current = null;
    setReceipt(null);
    setOperation("idle");
    setValidationStage(models.length > 0 ? "models_confirmed" : "idle");
  }

  function changeConnection(field: "baseUrl" | "apiKey", value: string) {
    clearReceipt();
    setModels([]);
    setDefaultModel("");
    setValidationStage("idle");
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
    setModels([]);
    setDefaultModel("");
    setValidationStage("idle");
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
      setValidationStage("models_confirmed");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setModels([]);
      setDefaultModel("");
      setValidationStage("idle");
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
      setValidationStage("complete");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation("idle");
      setValidationStage("models_confirmed");
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
    if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    receiptRef.current = null;
    setName("");
    setBaseUrl("");
    setApiKey("");
    setShowKey(false);
    setModels([]);
    setDefaultModel("");
    setReceipt(null);
    setFailure(null);
    setOperation("idle");
    setValidationStage("idle");
  }

  async function copyApiKey() {
    try {
      await navigator.clipboard.writeText(apiKey);
      setCopyStatus(providerMessages.copied);
    } catch {
      setCopyStatus(providerMessages.copyFailed);
    }
  }

  const busy = operation === "discovering" || operation === "validating" || operation === "saving";
  const canDiscover = baseUrl.trim().length > 0 && apiKey.length > 0 && !busy;
  const canValidate = models.length > 0 && defaultModel.length > 0 && !busy;
  const canSave = operation === "verified" && name.trim().length > 0;
  const errorId = failure ? "provider-validation-error" : undefined;

  return (
    <>
      <header className="page-header">
        <div>
          <h1>{providerMessages.pageTitle}</h1>
          <p>{providerMessages.pageSubtitle}</p>
        </div>
        <button className="command-button compact" type="button" onClick={resetEditor} disabled={busy}>
          <Plus size={17} aria-hidden="true" />
          {providerMessages.newProvider}
        </button>
      </header>

      <div className="provider-workspace">
        <section className="provider-list-pane" aria-labelledby="provider-list-heading">
          <div className="pane-heading">
            <h2 id="provider-list-heading">{providerMessages.verifiedProviders}</h2>
            <span>{providers.length}</span>
          </div>
          {listState === "loading" && <p className="pane-note">{providerMessages.loadingCatalog}</p>}
          {listState === "error" && <p className="inline-error">{providerMessages.catalogUnavailable}</p>}
          {listState === "ready" && providers.length === 0 && (
            <div className="empty-list">
              <Server size={22} aria-hidden="true" />
              <p>{providerMessages.emptyCatalog}</p>
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
              <h2 id="provider-editor-heading">{providerMessages.newProvider}</h2>
              <span>{providerMessages.editorSubtitle}</span>
            </div>
          </div>

          <div className="field-grid">
            <label className="form-field full-width">
              <span>{providerMessages.providerName}</span>
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={operation === "saving"}
                aria-describedby={errorId}
              />
            </label>
            <label className="form-field full-width">
              <span>{providerMessages.baseUrl}</span>
              <input
                type="url"
                value={baseUrl}
                onChange={(event) => changeConnection("baseUrl", event.target.value)}
                placeholder="https://provider.example/v1"
                disabled={busy}
                aria-describedby={errorId}
              />
            </label>
            <div className="form-field full-width">
              <label htmlFor="provider-api-key">{providerMessages.apiKey}</label>
              <div className="secret-input">
                <input
                  id="provider-api-key"
                  type={showKey ? "text" : "password"}
                  value={apiKey}
                  onChange={(event) => changeConnection("apiKey", event.target.value)}
                  autoComplete="off"
                  disabled={busy}
                  aria-describedby={errorId}
                />
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => setShowKey((current) => !current)}
                  aria-label={showKey ? providerMessages.hideApiKey : providerMessages.showApiKey}
                  title={showKey ? providerMessages.hideApiKey : providerMessages.showApiKey}
                  disabled={!apiKey}
                >
                  {showKey ? <EyeOff size={17} /> : <Eye size={17} />}
                </button>
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => void copyApiKey()}
                  aria-label={providerMessages.copyApiKey}
                  title={providerMessages.copyApiKey}
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
              <span>{providerMessages.defaultModel}</span>
              <select
                value={defaultModel}
                onChange={(event) => changeModel(event.target.value)}
                disabled={models.length === 0 || busy}
                aria-describedby={errorId}
              >
                <option value="">{providerMessages.chooseModel}</option>
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
              {providerMessages.discoverModels}
            </button>
          </div>

          <ValidationStatus
            modelsReady={models.length > 0}
            operation={operation}
            stage={validationStage}
            failure={failure}
          />

          <div className="editor-actions">
            {operation === "discovering" || operation === "validating" ? (
              <button className="secondary-button" type="button" onClick={() => void cancelCurrentRequest()}>
                <X size={17} aria-hidden="true" />
                {providerMessages.cancelRequest}
              </button>
            ) : (
              <button
                className="secondary-button"
                type="button"
                onClick={() => void runValidation()}
                disabled={!canValidate}
              >
                <ShieldCheck size={17} aria-hidden="true" />
                {providerMessages.validateProvider}
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
              {providerMessages.save}
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
  stage,
  failure,
}: {
  modelsReady: boolean;
  operation: Operation;
  stage: ProviderValidationStage | "idle" | "complete";
  failure: ProviderFailure | null;
}) {
  const verified = operation === "verified" || operation === "saving";
  return (
    <div className="validation-panel" aria-live="polite">
      <div className="validation-title">
        <KeyRound size={18} aria-hidden="true" />
        <strong>
          {verified ? providerMessages.validationPassed : providerMessages.validationTitle}
        </strong>
      </div>
      <ol className="validation-steps">
        <ValidationStep
          complete={modelsReady}
          active={operation === "discovering"}
          label={providerMessages.modelsConfirmed}
        />
        <ValidationStep
          complete={stage === "tool_round_trip" || stage === "complete"}
          active={stage === "responses_stream"}
          label={providerMessages.responsesStream}
        />
        <ValidationStep
          complete={stage === "complete"}
          active={stage === "tool_round_trip"}
          label={providerMessages.toolRoundTrip}
        />
      </ol>
      {failure && (
        <div className="validation-error" id="provider-validation-error" role="alert">
          <p>
            {providerFailureMessages[failure.messageId] ?? providerMessages.validationFallback}
          </p>
          <details>
            <summary>{providerMessages.technicalDetails}</summary>
            <code>{failure.category}</code>
          </details>
        </div>
      )}
    </div>
  );
}

function ValidationStep({ complete, active, label }: { complete: boolean; active: boolean; label: string }) {
  return (
    <li
      className={complete ? "is-complete" : active ? "is-active" : undefined}
      aria-current={active ? "step" : undefined}
    >
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
