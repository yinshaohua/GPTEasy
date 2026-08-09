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
  RefreshCw,
  Server,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";

import {
  asProviderFailure,
  cancelProviderRequest,
  copyProviderApiKey,
  deleteProvider,
  discoverProviderModels,
  discoverProviderModelsForUpdate,
  discardProviderValidation,
  listProviders,
  onProviderValidationProgress,
  renameProvider,
  revealProviderApiKey,
  revalidateProvider,
  saveProviderUpdate,
  saveAndApplyProviderUpdate,
  saveVerifiedProvider,
  validateProvider,
  validateProviderUpdate,
  type ProviderFailure,
  type ProviderSummary,
  type ProviderValidationReceipt,
  type ProviderValidationStage,
} from "./contracts/provider";
import { providerFailureMessages, providerMessages } from "./messages";

type Operation =
  | "idle"
  | "discovering"
  | "validating"
  | "revalidating"
  | "verified"
  | "saving"
  | "deleting";

export default function ProviderPage() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [listState, setListState] = useState<"loading" | "ready" | "error">("loading");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKeyPresent, setApiKeyPresent] = useState(false);
  const [apiKeyReplacement, setApiKeyReplacement] = useState(false);
  const [showKey, setShowKey] = useState(false);
  const [models, setModels] = useState<string[]>([]);
  const [defaultModel, setDefaultModel] = useState("");
  const [operation, setOperation] = useState<Operation>("idle");
  const [failure, setFailure] = useState<ProviderFailure | null>(null);
  const [receipt, setReceipt] = useState<ProviderValidationReceipt | null>(null);
  const [validationStage, setValidationStage] = useState<
    ProviderValidationStage | "idle" | "complete"
  >("idle");
  const [copyStatus, setCopyStatus] = useState("");
  const apiKeyRef = useRef<HTMLInputElement | null>(null);
  const activeRequest = useRef<string | null>(null);
  const receiptRef = useRef<string | null>(null);
  const selected = providers.find((provider) => provider.id === selectedId) ?? null;

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

  function discardReceipt(nextStage: ProviderValidationStage | "idle" = "idle") {
    if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    receiptRef.current = null;
    setReceipt(null);
    setOperation("idle");
    setValidationStage(nextStage);
  }

  function clearSecretInput() {
    if (apiKeyRef.current) apiKeyRef.current.value = "";
    setApiKeyPresent(false);
    setApiKeyReplacement(false);
    setShowKey(false);
    setCopyStatus("");
  }

  function selectProvider(provider: ProviderSummary) {
    discardReceipt("models_confirmed");
    setSelectedId(provider.id);
    setName(provider.name);
    setBaseUrl(provider.baseUrl);
    setModels([provider.defaultModel]);
    setDefaultModel(provider.defaultModel);
    setFailure(null);
    clearSecretInput();
  }

  function resetEditor() {
    discardReceipt();
    setSelectedId(null);
    setName("");
    setBaseUrl("");
    setModels([]);
    setDefaultModel("");
    setFailure(null);
    clearSecretInput();
  }

  function changeConnection(field: "baseUrl" | "apiKey", value: string) {
    discardReceipt();
    setModels([]);
    setDefaultModel("");
    setFailure(null);
    if (field === "baseUrl") {
      setBaseUrl(value);
    } else {
      setApiKeyPresent(value.length > 0);
      setApiKeyReplacement(selected !== null && value.length > 0);
    }
  }

  function changeModel(value: string) {
    discardReceipt("models_confirmed");
    setDefaultModel(value);
    setFailure(null);
  }

  function apiKeyForRequest(): string | null {
    const value = apiKeyRef.current?.value ?? "";
    if (!selected) return value;
    return apiKeyReplacement ? value : null;
  }

  async function discoverModels() {
    discardReceipt();
    setModels([]);
    setDefaultModel("");
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("discovering");
    setFailure(null);
    try {
      const apiKey = apiKeyForRequest();
      const result = selected
        ? await discoverProviderModelsForUpdate(
            requestId,
            selected.id,
            baseUrl,
            apiKey,
          )
        : await discoverProviderModels(requestId, baseUrl, apiKey ?? "");
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
    discardReceipt("models_confirmed");
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("validating");
    setFailure(null);
    try {
      const apiKey = apiKeyForRequest();
      const result = selected
        ? await validateProviderUpdate(
            requestId,
            selected.id,
            baseUrl,
            apiKey,
            defaultModel,
          )
        : await validateProvider(requestId, baseUrl, apiKey ?? "", defaultModel);
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

  async function runRevalidation() {
    if (!selected) return;
    discardReceipt("models_confirmed");
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("revalidating");
    setFailure(null);
    try {
      const updated = await revalidateProvider(requestId, selected.id);
      replaceProvider(updated);
      setOperation("idle");
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
    setOperation("saving");
    setFailure(null);
    try {
      let saved: ProviderSummary;
      if (!selected) {
        if (!receipt) return;
        saved = await saveVerifiedProvider(receipt.validationId, name);
        receiptRef.current = null;
        setProviders((current) => sortProviders([...current, saved]));
        resetEditor();
        return;
      }
      if (criticalDirty) {
        if (!receipt) return;
        if (selected.isCurrent) {
          try {
            saved = await saveAndApplyProviderUpdate(
              receipt.validationId,
              selected.id,
              name,
              false,
            );
          } catch (error) {
            const providerFailure = asProviderFailure(error);
            if (providerFailure.messageId !== "environment.consumer_confirmation_required") {
              throw error;
            }
            if (!window.confirm(providerMessages.consumerRiskConfirmation)) {
              setOperation("verified");
              return;
            }
            saved = await saveAndApplyProviderUpdate(
              receipt.validationId,
              selected.id,
              name,
              true,
            );
          }
        } else {
          saved = await saveProviderUpdate(receipt.validationId, selected.id, name);
        }
        receiptRef.current = null;
      } else {
        saved = await renameProvider(selected.id, name);
      }
      replaceProvider(saved);
      setName(saved.name);
      setBaseUrl(saved.baseUrl);
      setModels([saved.defaultModel]);
      setDefaultModel(saved.defaultModel);
      clearSecretInput();
      setReceipt(null);
      setOperation("idle");
      setValidationStage("models_confirmed");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation(receipt ? "verified" : "idle");
    }
  }

  async function deleteSelectedProvider() {
    if (!selected || selected.isCurrent || !window.confirm(providerMessages.deleteConfirmation)) {
      return;
    }
    setOperation("deleting");
    setFailure(null);
    try {
      await deleteProvider(selected.id);
      setProviders((current) => current.filter((provider) => provider.id !== selected.id));
      resetEditor();
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation("idle");
    }
  }

  async function toggleApiKey() {
    if (showKey) {
      setShowKey(false);
      return;
    }
    if (selected && !apiKeyReplacement && !apiKeyRef.current?.value) {
      try {
        const secret = await revealProviderApiKey(selected.id);
        if (apiKeyRef.current) apiKeyRef.current.value = secret.value;
      } catch (error) {
        setFailure(asProviderFailure(error));
        return;
      }
    }
    setShowKey(true);
  }

  async function copyApiKey() {
    try {
      if (selected && !apiKeyReplacement) {
        await copyProviderApiKey(selected.id);
      } else {
        await navigator.clipboard.writeText(apiKeyRef.current?.value ?? "");
      }
      setCopyStatus(providerMessages.copied);
    } catch {
      setCopyStatus(providerMessages.copyFailed);
    }
  }

  function replaceProvider(updated: ProviderSummary) {
    setProviders((current) =>
      sortProviders(current.map((provider) => (provider.id === updated.id ? updated : provider))),
    );
  }

  const busy = ["discovering", "validating", "revalidating", "saving", "deleting"].includes(
    operation,
  );
  const nameDirty = selected !== null && name.trim() !== selected.name;
  const criticalDirty =
    selected !== null &&
    (baseUrl.trim() !== selected.baseUrl ||
      defaultModel !== selected.defaultModel ||
      apiKeyReplacement);
  const canDiscover =
    baseUrl.trim().length > 0 && (selected !== null || apiKeyPresent) && !busy;
  const canValidate =
    models.length > 0 &&
    defaultModel.length > 0 &&
    (!selected || criticalDirty) &&
    !busy;
  const canSave =
    name.trim().length > 0 &&
    !busy &&
    (selected
      ? criticalDirty
        ? operation === "verified"
        : nameDirty
      : operation === "verified");
  const canRevalidate = selected !== null && !nameDirty && !criticalDirty && !busy;
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
              <button
                className="provider-list-row"
                type="button"
                key={provider.id}
                onClick={() => selectProvider(provider)}
                aria-label={`编辑 ${provider.name}`}
                aria-pressed={selectedId === provider.id}
                disabled={busy}
              >
                <span className="provider-list-name">
                  <strong>{provider.name}</strong>
                  {provider.isCurrent && <span className="current-badge">{providerMessages.currentProvider}</span>}
                </span>
                <span title={provider.defaultModel}>{provider.defaultModel}</span>
                <time dateTime={new Date(provider.verifiedAtEpochSeconds * 1000).toISOString()}>
                  {formatVerificationTime(provider.verifiedAtEpochSeconds)}
                </time>
              </button>
            ))}
          </div>
        </section>

        <section className="provider-editor" aria-labelledby="provider-editor-heading">
          <div className="pane-heading editor-heading">
            <div>
              <h2 id="provider-editor-heading">{selected?.name ?? providerMessages.newProvider}</h2>
              <span>{selected ? providerMessages.detailsSubtitle : providerMessages.editorSubtitle}</span>
            </div>
            {selected?.isCurrent && <span className="current-badge">{providerMessages.currentProvider}</span>}
          </div>

          <div className="field-grid">
            <label className="form-field full-width">
              <span>{providerMessages.providerName}</span>
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={operation === "saving" || operation === "deleting"}
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
                  ref={apiKeyRef}
                  id="provider-api-key"
                  type={showKey ? "text" : "password"}
                  defaultValue=""
                  onChange={(event) => changeConnection("apiKey", event.target.value)}
                  placeholder={selected ? providerMessages.savedApiKey : undefined}
                  autoComplete="off"
                  disabled={busy}
                  aria-describedby={errorId}
                />
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => void toggleApiKey()}
                  aria-label={showKey ? providerMessages.hideApiKey : providerMessages.showApiKey}
                  title={showKey ? providerMessages.hideApiKey : providerMessages.showApiKey}
                  disabled={!selected && !apiKeyPresent}
                >
                  {showKey ? <EyeOff size={17} /> : <Eye size={17} />}
                </button>
                <button
                  className="field-icon-button"
                  type="button"
                  onClick={() => void copyApiKey()}
                  aria-label={providerMessages.copyApiKey}
                  title={providerMessages.copyApiKey}
                  disabled={!selected && !apiKeyPresent}
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

          {selected && (
            <div className="record-actions">
              <button
                className="secondary-button"
                type="button"
                onClick={() => void runRevalidation()}
                disabled={!canRevalidate}
              >
                <RefreshCw
                  className={operation === "revalidating" ? "is-spinning" : undefined}
                  size={17}
                  aria-hidden="true"
                />
                {providerMessages.revalidate}
              </button>
              <button
                className="danger-button"
                type="button"
                onClick={() => void deleteSelectedProvider()}
                disabled={selected.isCurrent || busy}
              >
                <Trash2 size={17} aria-hidden="true" />
                {providerMessages.deleteProvider}
              </button>
            </div>
          )}

          <div className="editor-actions">
            {operation === "discovering" || operation === "validating" || operation === "revalidating" ? (
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
                {selected ? providerMessages.validateUpdate : providerMessages.validateProvider}
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
              {selected?.isCurrent && criticalDirty ? providerMessages.saveAndApply : providerMessages.save}
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
  const verified = stage === "complete" || operation === "verified" || operation === "saving";
  return (
    <div className="validation-panel" aria-live="polite">
      <div className="validation-title">
        <KeyRound size={18} aria-hidden="true" />
        <strong>{verified ? providerMessages.validationPassed : providerMessages.validationTitle}</strong>
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
          <p>{providerFailureMessages[failure.messageId] ?? providerMessages.validationFallback}</p>
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

function sortProviders(providers: ProviderSummary[]): ProviderSummary[] {
  return [...providers].sort((left, right) => left.name.localeCompare(right.name, "zh-CN"));
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
