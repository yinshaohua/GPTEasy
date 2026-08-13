import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Check,
  CheckCircle2,
  Copy,
  Eye,
  EyeOff,
  ExternalLink,
  GripVertical,
  LoaderCircle,
  LogIn,
  Pencil,
  Plus,
  RefreshCw,
  RotateCcw,
  Server,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";

import ProviderValidationDialog, {
  type ProviderValidationSession,
  type ProviderValidationSource,
} from "./ProviderValidationDialog";

import {
  asProviderFailure,
  cancelProviderRequest,
  confirmProviderValidationBaseUrl,
  copyProviderApiKey,
  deleteProvider,
  discoverProviderModels,
  discoverProviderModelsForUpdate,
  discardProviderValidation,
  listProviders,
  openDaywayWebsite,
  onProviderValidationProgress,
  renameProvider,
  reorderProviders,
  revealProviderApiKey,
  revalidateProvider,
  saveProviderUpdate,
  saveAndApplyProviderUpdate,
  saveDaywayProvider,
  saveVerifiedProvider,
  validateProvider,
  validateProviderUpdate,
  type ProviderFailure,
  type ProviderSummary,
  type ProviderValidationReceipt,
  type ProviderValidationStage,
} from "./contracts/provider";
import {
  applyEnvironmentProvider,
  asEnvironmentFailure,
  getEnvironmentSnapshot,
  restoreLastEnvironmentConfig,
  switchToOpenAiLogin,
  type EnvironmentFailure,
  type EnvironmentSnapshot,
} from "./contracts/environment";
import {
  authenticationModeMessages,
  consumerStatusMessages,
  environmentStateMessages,
  providerFailureMessages,
  providerMessages,
} from "./messages";

type Operation =
  | "idle"
  | "discovering"
  | "validating"
  | "revalidating"
  | "verified"
  | "saving"
  | "deleting";

type PageView = "catalog" | "detail";
type Confirmation = "discard" | "validation" | null;

const DAYWAY_NAME = "DayWay";
const DAYWAY_BASE_URL = "https://dayway.site/v1";

export default function ProviderPage() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [listState, setListState] = useState<"loading" | "ready" | "error">("loading");
  const [environment, setEnvironment] = useState<EnvironmentSnapshot | null>(null);
  const [environmentState, setEnvironmentState] = useState<"loading" | "ready" | "error">("loading");
  const [environmentFailure, setEnvironmentFailure] = useState<EnvironmentFailure | null>(null);
  const [environmentOperation, setEnvironmentOperation] = useState<"idle" | "restoring" | "switching_mode">("idle");
  const [view, setView] = useState<PageView>("catalog");
  const [confirmation, setConfirmation] = useState<Confirmation>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [isRecommendedCandidate, setIsRecommendedCandidate] = useState(false);
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
  const [validationSession, setValidationSession] = useState<ProviderValidationSession | null>(null);
  const [addressSuggestion, setAddressSuggestion] = useState<ProviderValidationReceipt | null>(null);
  const [catalogFeedback, setCatalogFeedback] = useState("");
  const [copyStatus, setCopyStatus] = useState("");
  const apiKeyRef = useRef<HTMLInputElement | null>(null);
  const activeRequest = useRef<string | null>(null);
  const draggedProviderId = useRef<string | null>(null);
  const receiptRef = useRef<string | null>(null);
  const selected = providers.find((provider) => provider.id === selectedId) ?? null;
  const savedDayway = providers.find((provider) => provider.recommendationId === "dayway") ?? null;
  const isDaywayEditor = isRecommendedCandidate || selected?.recommendationId === "dayway";

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
    void getEnvironmentSnapshot()
      .then((snapshot) => {
        if (mounted) {
          setEnvironment(snapshot);
          setEnvironmentState("ready");
        }
      })
      .catch(() => {
        if (mounted) setEnvironmentState("error");
      });
    return () => {
      mounted = false;
      if (activeRequest.current) void cancelProviderRequest(activeRequest.current);
      if (receiptRef.current) void discardProviderValidation(receiptRef.current);
    };
  }, []);

  useEffect(() => {
    if (!catalogFeedback) return;
    const timer = window.setTimeout(() => setCatalogFeedback(""), 5_000);
    return () => window.clearTimeout(timer);
  }, [catalogFeedback]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    void onProviderValidationProgress((progress) => {
      if (progress.requestId === activeRequest.current) {
        setValidationStage(progress.stage);
        setValidationSession((current) => current?.status === "running"
          ? { ...current, stage: progress.stage, stageStartedAt: Date.now() }
          : current);
      }
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
    setAddressSuggestion(null);
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

  function editProvider(provider: ProviderSummary) {
    discardReceipt("models_confirmed");
    setSelectedId(provider.id);
    setIsRecommendedCandidate(false);
    setName(provider.name);
    setBaseUrl(provider.baseUrl);
    setModels([provider.defaultModel]);
    setDefaultModel(provider.defaultModel);
    setFailure(null);
    clearSecretInput();
    setView("detail");
  }

  function resetEditor(nextView: PageView = "detail") {
    discardReceipt();
    setSelectedId(null);
    setIsRecommendedCandidate(false);
    setName("");
    setBaseUrl("");
    setModels([]);
    setDefaultModel("");
    setFailure(null);
    clearSecretInput();
    setView(nextView);
  }

  function configureDayway() {
    resetEditor("detail");
    setIsRecommendedCandidate(true);
    setName(DAYWAY_NAME);
    setBaseUrl(DAYWAY_BASE_URL);
  }

  function requestBack() {
    if (dirty) {
      setConfirmation("discard");
      return;
    }
    resetEditor("catalog");
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
    discardReceipt();
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("validating");
    setFailure(null);
    setValidationSession(createValidationSession({
      kind: "detail",
      providerName: name.trim(),
    }));
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
      setAddressSuggestion(
        result.requestedBaseUrl !== result.normalizedBaseUrl ? result : null,
      );
      setOperation("verified");
      setValidationStage("complete");
      setValidationSession((current) => current
        ? { ...current, status: "succeeded", stage: "tool_round_trip" }
        : current);
    } catch (error) {
      const providerFailure = asProviderFailure(error);
      setFailure(providerFailure);
      setOperation("idle");
      setValidationSession((current) => current
        ? { ...current, status: "failed", failure: providerFailure }
        : current);
    } finally {
      activeRequest.current = null;
    }
  }

  async function runRevalidation(provider?: ProviderSummary) {
    const target = provider ?? selected;
    if (!target) return;
    discardReceipt("models_confirmed");
    const requestId = createRequestId();
    activeRequest.current = requestId;
    setOperation("revalidating");
    setFailure(null);
    setCatalogFeedback("");
    setValidationSession(createValidationSession({
      kind: "catalog",
      providerName: target.name,
    }));
    try {
      const result = await revalidateProvider(requestId, target.id);
      replaceProvider(result.provider);
      if (result.validationReceipt) {
        editProvider(result.provider);
        receiptRef.current = result.validationReceipt.validationId;
        setReceipt(result.validationReceipt);
        setAddressSuggestion(result.validationReceipt);
        setOperation("verified");
        setValidationStage("complete");
      } else {
        setOperation("idle");
      }
      setValidationStage("complete");
      setValidationSession((current) => current
        ? { ...current, status: "succeeded", stage: "tool_round_trip" }
        : current);
    } catch (error) {
      const providerFailure = asProviderFailure(error);
      setFailure(providerFailure);
      setOperation("idle");
      setValidationSession((current) => current
        ? { ...current, status: "failed", failure: providerFailure }
        : current);
    } finally {
      activeRequest.current = null;
    }
  }

  async function cancelCurrentRequest() {
    if (activeRequest.current) await cancelProviderRequest(activeRequest.current);
  }

  function closeValidationSession() {
    if (!validationSession || validationSession.status === "running") return;
    if (validationSession.source.kind === "catalog") {
      setCatalogFeedback(validationSession.status === "succeeded"
        ? `${validationSession.source.providerName} 重新验证成功。`
        : `${validationSession.source.providerName} 最近验证失败。`);
      setFailure(null);
    }
    setValidationSession(null);
  }

  async function acceptAddressSuggestion() {
    if (!addressSuggestion) return;
    try {
      await confirmProviderValidationBaseUrl(
        addressSuggestion.validationId,
        addressSuggestion.normalizedBaseUrl,
      );
      setBaseUrl(addressSuggestion.normalizedBaseUrl);
      setAddressSuggestion(null);
    } catch (error) {
      discardReceipt("models_confirmed");
      setFailure(asProviderFailure(error));
    }
  }

  function rejectAddressSuggestion() {
    discardReceipt("models_confirmed");
  }

  async function saveProvider() {
    if ((!selected || criticalDirty) && !receipt) {
      setConfirmation("validation");
      return;
    }
    setOperation("saving");
    setFailure(null);
    try {
      let saved: ProviderSummary;
      if (!selected) {
        if (!receipt) return;
        if (isRecommendedCandidate) {
          try {
            saved = await saveDaywayProvider(receipt.validationId);
          } catch (error) {
            const providerFailure = asProviderFailure(error);
            if (providerFailure.messageId !== "provider.recommended_name_conflict") throw error;
            if (!window.confirm(providerMessages.daywayNameConflictConfirmation)) {
              setOperation("verified");
              return;
            }
            saved = await saveDaywayProvider(receipt.validationId, true);
          }
        } else {
          saved = await saveVerifiedProvider(receipt.validationId, name);
        }
        receiptRef.current = null;
        setProviders((current) => isRecommendedCandidate ? [saved, ...current] : [...current, saved]);
        resetEditor("catalog");
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
      setView("catalog");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation(receipt ? "verified" : "idle");
    }
  }

  async function deleteCatalogProvider(provider: ProviderSummary) {
    if (provider.isCurrent || !window.confirm(providerMessages.deleteConfirmation)) {
      return;
    }
    setOperation("deleting");
    setFailure(null);
    try {
      await deleteProvider(provider.id);
      setProviders((current) => current.filter((item) => item.id !== provider.id));
      if (selectedId === provider.id) resetEditor("catalog");
      setOperation("idle");
    } catch (error) {
      setFailure(asProviderFailure(error));
      setOperation("idle");
    }
  }

  async function switchCatalogProvider(provider: ProviderSummary) {
    if (provider.isCurrent || !environment || !canApplyProvider(environment)) return;
    setOperation("saving");
    setEnvironmentFailure(null);
    try {
      const confirmSwitchRisk =
        environment.mode === "openai_login" ||
        environment.requiresTakeoverConfirmation ||
        environment.requiresConsumerConfirmation;
      if (confirmSwitchRisk && !window.confirm(switchConfirmation(environment, provider))) {
        setOperation("idle");
        return;
      }
      const updated = await applyEnvironmentProvider(
        provider.id,
        confirmSwitchRisk,
        environment.revision,
      );
      setEnvironment(updated);
      setProviders((current) =>
        current.map((item) => ({
          ...item,
          isCurrent: item.id === updated.currentProvider?.id,
        })),
      );
    } catch (error) {
      const environmentFailure = asEnvironmentFailure(error);
      setEnvironmentFailure(environmentFailure);
    } finally {
      setOperation("idle");
    }
  }

  async function restoreLatest() {
    if (!environment || environment.restoreAvailability !== "available") return;
    const preview = environment.restorePreview;
    const artifacts = preview?.artifacts.map(artifactName).join("、") || "Codex 配置工件";
    const target = preview?.targetProvider
      ? `供应商“${preview.targetProvider.name}”`
      : preview?.targetMode === "openai_login"
        ? "OpenAI 登录模式"
        : "外部配置";
    if (!window.confirm(`将恢复 ${artifacts}，恢复后为${target}。是否继续？`)) return;
    setEnvironmentOperation("restoring");
    setEnvironmentFailure(null);
    try {
      const updated = await restoreLastEnvironmentConfig(true, environment.revision);
      applyEnvironmentSnapshot(updated);
    } catch (error) {
      setEnvironmentFailure(asEnvironmentFailure(error));
    } finally {
      setEnvironmentOperation("idle");
    }
  }

  async function enableOpenAiLogin() {
    if (!environment || environment.loginStatus !== "logged_in" || environment.mode === "openai_login") return;
    if (!window.confirm("将退出供应商模式并使用 Codex 已有的 OpenAI 登录；Codex 登录凭据不会被修改。是否继续？")) return;
    setEnvironmentOperation("switching_mode");
    setEnvironmentFailure(null);
    try {
      const updated = await switchToOpenAiLogin(true, environment.revision);
      applyEnvironmentSnapshot(updated);
    } catch (error) {
      setEnvironmentFailure(asEnvironmentFailure(error));
    } finally {
      setEnvironmentOperation("idle");
    }
  }

  function applyEnvironmentSnapshot(updated: EnvironmentSnapshot) {
    setEnvironment(updated);
    setProviders((current) => current.map((provider) => ({
      ...provider,
      isCurrent: provider.id === updated.currentProvider?.id,
    })));
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

  async function visitDaywayWebsite() {
    try {
      await openDaywayWebsite();
    } catch (error) {
      setFailure(asProviderFailure(error));
    }
  }

  function replaceProvider(updated: ProviderSummary) {
    setProviders((current) =>
      current.map((provider) => (provider.id === updated.id ? updated : provider)),
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
  const dirty = selected
    ? nameDirty || criticalDirty
    : isRecommendedCandidate || name.trim().length > 0 ||
      baseUrl.trim().length > 0 ||
      apiKeyPresent ||
      defaultModel.length > 0;
  const canDiscover =
    baseUrl.trim().length > 0 && (selected !== null || apiKeyPresent) && !busy;
  const canValidate =
    models.length > 0 &&
    defaultModel.length > 0 &&
    (!selected || criticalDirty) &&
    !busy;
  const canSave = name.trim().length > 0 && !busy && (selected ? dirty : true);
  const errorId = failure ? "provider-validation-error" : undefined;

  return (
    <>
      <header className="page-header">
        <div>
          <h1>{providerMessages.pageTitle}</h1>
          <p>{providerMessages.pageSubtitle}</p>
        </div>
        {view === "catalog" && (
          <button
            className="command-button compact"
            type="button"
            onClick={() => resetEditor("detail")}
            disabled={busy}
          >
            <Plus size={17} aria-hidden="true" />
            {providerMessages.newProvider}
          </button>
        )}
      </header>

      {view === "catalog" ? (
        <>
        <EnvironmentSummary
          state={environmentState}
          snapshot={environment}
        />
        <section className="provider-catalog" aria-labelledby="provider-catalog-heading">
          <div className="catalog-heading">
            <div>
              <h2 id="provider-catalog-heading">{providerMessages.catalogTitle}</h2>
              <span>{providers.length} 个已验证供应商</span>
            </div>
          </div>
          {listState === "loading" && <p className="pane-note">{providerMessages.loadingCatalog}</p>}
          {listState === "error" && <p className="inline-error">{providerMessages.catalogUnavailable}</p>}
          <div className="provider-list" aria-label={providerMessages.verifiedProviders}>
            {!savedDayway && (
              <article className="provider-list-row provider-template-row">
                <span className="provider-drag-placeholder" aria-hidden="true" />
                <div className="provider-row-summary">
                  <div className="provider-row-title">
                    <strong className="provider-row-name">{DAYWAY_NAME}</strong>
                    <span className="recommended-badge">推荐</span>
                    <span className="pending-badge">待配置</span>
                  </div>
                  <span className="provider-row-url" title={DAYWAY_BASE_URL}>{DAYWAY_BASE_URL}</span>
                  <span className="provider-row-model">尚未选择</span>
                </div>
                <div className="provider-row-actions">
                  <button className="secondary-button compact" type="button" onClick={() => void visitDaywayWebsite()} aria-label="访问 DayWay 官网">
                    <ExternalLink size={16} aria-hidden="true" />
                    访问官网
                  </button>
                  <button className="command-button compact" type="button" onClick={configureDayway} disabled={busy} aria-label="配置 DayWay">
                    <Pencil size={16} aria-hidden="true" />
                    配置
                  </button>
                </div>
              </article>
            )}
            {providers.map((provider) => (
              <article
                className="provider-list-row"
                key={provider.id}
                onDragOver={(event) => event.preventDefault()}
                onDrop={(event) => {
                  event.preventDefault();
                  const sourceId = draggedProviderId.current;
                  draggedProviderId.current = null;
                  if (!sourceId || sourceId === provider.id || busy || provider.recommendationId === "dayway") return;
                  const next = providers.filter((item) => item.id !== sourceId);
                  const targetIndex = next.findIndex((item) => item.id === provider.id);
                  if (targetIndex < 0) return;
                  next.splice(targetIndex, 0, providers.find((item) => item.id === sourceId)!);
                  setProviders(next);
                  void reorderProviders(next.map((item) => item.id)).catch(() => {
                    void listProviders().then(setProviders).catch(() => undefined);
                  });
                }}
              >
                {provider.recommendationId === "dayway" ? (
                  <span className="provider-drag-placeholder" aria-hidden="true" />
                ) : (
                  <span className="provider-drag-handle" draggable role="img" aria-label={`拖拽排序 ${provider.name}`} title={`拖拽排序 ${provider.name}`} onDragStart={() => { draggedProviderId.current = provider.id; }} onDragEnd={() => { draggedProviderId.current = null; }}>
                    <GripVertical size={18} aria-hidden="true" />
                  </span>
                )}
                <div className="provider-row-summary">
                  <div className="provider-row-title">
                    <strong className="provider-row-name">{provider.name}</strong>
                    {provider.recommendationId === "dayway" && <span className="recommended-badge">推荐</span>}
                    {provider.hasRecommendationUpdate && <span className="pending-badge">推荐地址已更新</span>}
                    <span className="verified-badge">{providerMessages.verified}</span>
                    {provider.isCurrent && (
                      <span className="current-badge">{providerMessages.currentProvider}</span>
                    )}
                    <span className="provider-verified-time">
                      验证于 {formatVerifiedAt(provider.verifiedAtEpochSeconds)}
                    </span>
                  </div>
                  <span className="provider-row-url" title={provider.baseUrl}>{provider.baseUrl}</span>
                  <span className="provider-row-model" title={provider.defaultModel}>
                    {provider.defaultModel}
                  </span>
                </div>
                <div className="provider-row-actions">
                  {provider.recommendationId === "dayway" && (
                    <button className="secondary-button compact" type="button" onClick={() => void visitDaywayWebsite()} aria-label="访问 DayWay 官网">
                      <ExternalLink size={16} aria-hidden="true" />
                      访问官网
                    </button>
                  )}
                  <button
                    className="secondary-button compact"
                    type="button"
                    onClick={() => void runRevalidation(provider)}
                    disabled={busy}
                    aria-label={`验证 ${provider.name}`}
                  >
                    <RefreshCw size={16} aria-hidden="true" />
                    验证
                  </button>
                  <button
                    className="secondary-button compact"
                    type="button"
                    onClick={() => editProvider(provider)}
                    disabled={busy}
                    aria-label={`修改 ${provider.name}`}
                  >
                    <Pencil size={16} aria-hidden="true" />
                    {providerMessages.editProvider}
                  </button>
                  <button
                    className="danger-button compact"
                    type="button"
                    onClick={() => void deleteCatalogProvider(provider)}
                    disabled={provider.isCurrent || busy}
                    aria-label={`删除 ${provider.name}`}
                  >
                    <Trash2 size={16} aria-hidden="true" />
                    {providerMessages.deleteProvider}
                  </button>
                  <button
                    className="command-button compact"
                    type="button"
                    onClick={() => void switchCatalogProvider(provider)}
                    disabled={provider.isCurrent || busy || !environment || !canApplyProvider(environment)}
                    aria-label={provider.isCurrent ? `${provider.name} 当前使用` : `切换到 ${provider.name}`}
                  >
                    <Check size={16} aria-hidden="true" />
                    {provider.isCurrent ? providerMessages.currentProvider : providerMessages.switchProvider}
                  </button>
                </div>
              </article>
            ))}
            {listState === "ready" && providers.length === 0 && (
              <p className="empty-catalog-note">{providerMessages.emptyCatalog}</p>
            )}
          </div>
          {environment?.state === "conflict" && environment.takeoverAvailable === false && (
            <p className="inline-error">无法安全解析当前配置，不能强制覆盖。</p>
          )}
          {catalogFeedback && <p className="catalog-feedback" role="status">{catalogFeedback}</p>}
        </section>
        <EnvironmentActions
          snapshot={environment}
          failure={environmentFailure}
          busy={busy || environmentOperation !== "idle"}
          restoring={environmentOperation === "restoring"}
          switchingMode={environmentOperation === "switching_mode"}
          onRestore={() => void restoreLatest()}
          onSwitchMode={() => void enableOpenAiLogin()}
        />
        </>
      ) : (
        <section className="provider-detail" aria-labelledby="provider-editor-heading">
          <div className="detail-heading">
            <div>
              <h2 id="provider-editor-heading">
                {selected ? `修改 ${selected.name}` : isRecommendedCandidate ? "配置 DayWay" : providerMessages.newProvider}
              </h2>
              <span>{selected ? providerMessages.detailsSubtitle : providerMessages.editorSubtitle}</span>
            </div>
            {selected?.isCurrent && (
              <span className="current-badge">{providerMessages.currentProvider}</span>
            )}
          </div>

          <div className="field-grid provider-detail-fields">
            <label className="form-field full-width">
              <span>{providerMessages.providerName}</span>
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                disabled={operation === "saving" || isDaywayEditor}
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
              {selected?.hasRecommendationUpdate && baseUrl !== DAYWAY_BASE_URL && (
                <button
                  className="secondary-button recommended-address-action"
                  type="button"
                  onClick={() => changeConnection("baseUrl", DAYWAY_BASE_URL)}
                  disabled={busy}
                  aria-label="采用 DayWay 推荐地址"
                >
                  采用推荐地址
                </button>
              )}
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
                {models.map((model) => <option key={model} value={model}>{model}</option>)}
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

          {failure && !validationSession && (
            <p className="inline-error" id="provider-validation-error" role="alert">
              {providerFailureMessages[failure.messageId] ?? providerMessages.validationFallback}
            </p>
          )}

          <div className="detail-actions">
            <div className="detail-validation-action">
              {operation === "discovering" || operation === "validating" ? (
                <button className="secondary-button" type="button" onClick={() => void cancelCurrentRequest()}>
                  <X size={17} aria-hidden="true" />
                  {operation === "validating"
                    ? providerMessages.cancelValidation
                    : providerMessages.cancelRequest}
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
              <span role="status">{candidateStatus(operation, validationStage, failure)}</span>
            </div>
            <div className="detail-navigation-actions">
              <button className="secondary-button" type="button" onClick={requestBack} disabled={busy}>
                {providerMessages.back}
              </button>
              <button className="command-button" type="button" onClick={() => void saveProvider()} disabled={!canSave}>
                {operation === "saving" ? (
                  <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
                ) : (
                  <Check size={17} aria-hidden="true" />
                )}
                {selected?.isCurrent && criticalDirty ? providerMessages.saveAndApply : providerMessages.save}
              </button>
            </div>
          </div>
        </section>
      )}

      {confirmation === "discard" && (
        <ConfirmationDialog
          title={providerMessages.unsavedTitle}
          message={providerMessages.unsavedMessage}
          primaryLabel={providerMessages.discardChanges}
          secondaryLabel={providerMessages.continueEditing}
          onPrimary={() => {
            setConfirmation(null);
            resetEditor("catalog");
          }}
          onSecondary={() => setConfirmation(null)}
          danger
        />
      )}
      {confirmation === "validation" && (
        <ConfirmationDialog
          title={providerMessages.validationRequiredTitle}
          message={providerMessages.validationRequiredMessage}
          primaryLabel={providerMessages.startValidation}
          secondaryLabel={providerMessages.continueEditing}
          onPrimary={() => {
            setConfirmation(null);
            void runValidation();
          }}
          onSecondary={() => setConfirmation(null)}
          primaryDisabled={!canValidate}
        />
      )}
      {validationSession && (
        <ProviderValidationDialog
          session={validationSession}
          onCancel={() => void cancelCurrentRequest()}
          onClose={closeValidationSession}
        />
      )}
      {addressSuggestion && !validationSession && (
        <AddressSuggestionDialog
          requestedBaseUrl={addressSuggestion.requestedBaseUrl}
          suggestedBaseUrl={addressSuggestion.normalizedBaseUrl}
          onAccept={() => void acceptAddressSuggestion()}
          onReject={rejectAddressSuggestion}
        />
      )}
    </>
  );
}

const restoreAvailabilityMessages = {
  available: "可恢复到最近一次 GPTEasy 修改前的配置。",
  no_backup: "尚无可恢复的 GPTEasy 配置修改。",
  artifacts_changed: "受管工件在最近一次修改后发生变化，恢复已禁用。",
  invalid_backup: "最近一次配置备份不完整，恢复已禁用。",
  recovery_pending: "恢复协调完成前不能再次恢复。",
} as const;

function EnvironmentSummary({
  state,
  snapshot,
}: {
  state: "loading" | "ready" | "error";
  snapshot: EnvironmentSnapshot | null;
}) {
  if (state === "loading") return <p className="environment-status-note">正在读取当前用户 Codex 环境</p>;
  if (state === "error" || !snapshot) {
    return <p className="environment-status-note is-error" role="alert">无法读取当前用户 Codex 环境。</p>;
  }
  const title = snapshot.mode
    ? authenticationModeMessages[snapshot.mode]
    : environmentStateMessages[snapshot.state];
  return (
    <section className={`environment-status-bar is-${snapshot.state}`} aria-labelledby="environment-status-heading">
      {snapshot.state === "managed" ? <CheckCircle2 size={20} aria-hidden="true" /> : <AlertTriangle size={20} aria-hidden="true" />}
      <div className="environment-status-copy">
        <h2 id="environment-status-heading">{title}</h2>
        <span>{snapshot.currentProvider ? `当前供应商：${snapshot.currentProvider.name}` : environmentDescription(snapshot)}</span>
      </div>
      <dl className="environment-status-facts">
        <div><dt>桌面版</dt><dd>{consumerStatusMessages[snapshot.consumers?.desktop ?? "unknown"]}</dd></div>
        <div><dt>Codex CLI</dt><dd>{consumerStatusMessages[snapshot.consumers?.cli ?? "unknown"]}</dd></div>
        <div><dt>待重启</dt><dd>{snapshot.pendingRestart ? "是" : "否"}</dd></div>
      </dl>
    </section>
  );
}

function EnvironmentActions({
  snapshot,
  failure,
  busy,
  restoring,
  switchingMode,
  onRestore,
  onSwitchMode,
}: {
  snapshot: EnvironmentSnapshot | null;
  failure: EnvironmentFailure | null;
  busy: boolean;
  restoring: boolean;
  switchingMode: boolean;
  onRestore: () => void;
  onSwitchMode: () => void;
}) {
  const restoreAvailability = snapshot?.restoreAvailability ?? "no_backup";
  const openAiReason = !snapshot
    ? "环境状态不可用。"
    : snapshot.mode === "openai_login"
      ? "当前已是 OpenAI 登录模式。"
      : snapshot.loginStatus === "not_logged_in"
        ? "请先在 Codex 中完成 OpenAI 登录。"
        : snapshot.loginStatus === "unavailable"
          ? "无法确认 Codex 登录状态，已阻止切换。"
          : "使用 Codex 已有的 OpenAI 登录。";
  return (
    <section className="environment-tools" aria-label="Codex 环境操作">
      <div className="environment-tool">
        <button className="secondary-button" type="button" onClick={onRestore} disabled={busy || restoreAvailability !== "available"}>
          {restoring ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <RotateCcw size={17} aria-hidden="true" />}
          恢复上次配置
        </button>
        <span>{restoring ? "正在恢复上次配置。" : restoreAvailabilityMessages[restoreAvailability]}</span>
      </div>
      <div className="environment-tool">
        <button className="secondary-button" type="button" onClick={onSwitchMode} disabled={busy || !snapshot || snapshot.mode === "openai_login" || snapshot.loginStatus !== "logged_in"}>
          {switchingMode ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <LogIn size={17} aria-hidden="true" />}
          切换到 OpenAI 登录模式
        </button>
        <span>{openAiReason}</span>
      </div>
      <button className="secondary-button upcoming-command" type="button" disabled>选择 WSL2 供应商 <span>即将支持</span></button>
      <button className="secondary-button upcoming-command" type="button" disabled>导出 Linux 脚本 <span>即将支持</span></button>
      {failure && <p className="inline-error environment-tool-error" role="alert">{environmentFailureMessage(failure.messageId)}</p>}
    </section>
  );
}

function environmentDescription(snapshot: EnvironmentSnapshot): string {
  if (snapshot.mode === "openai_login") {
    return snapshot.loginStatus === "logged_in"
      ? "Codex 已有本地 OpenAI 登录凭据。"
      : snapshot.loginStatus === "not_logged_in"
        ? "OpenAI 登录已在外部失效；当前模式保持不变。"
        : "无法确认 OpenAI 登录状态；当前模式保持不变。";
  }
  return snapshot.state === "conflict"
    ? "配置所有权无法安全确认。"
    : "尚未建立有效的 GPTEasy 供应商 ID。";
}

function switchConfirmation(snapshot: EnvironmentSnapshot, provider: ProviderSummary): string {
  const context = snapshot.mode === "openai_login"
    ? "将退出 OpenAI 登录模式"
    : snapshot.state === "external"
      ? "将接管外部配置"
      : snapshot.state === "conflict"
        ? "将重新接管管理冲突"
        : "将切换当前供应商";
  const impacts = snapshot.impacts.map((impact) => `${artifactName(impact.artifact)}：${impact.fields.join("、")}`).join("；");
  const desktop = snapshot.consumers?.desktop ?? "unknown";
  const cli = snapshot.consumers?.cli ?? "unknown";
  const desktopRisk = desktop === "running" ? "ChatGPT/Codex 桌面版正在运行" : desktop === "unknown" ? "无法确认桌面版状态" : "桌面版未运行";
  const cliRisk = cli === "running" ? "Codex CLI 正在运行且不会被关闭" : cli === "unknown" ? "无法确认 Codex CLI 状态" : "Codex CLI 未运行";
  return `${context}并应用“${provider.name}”。将修改：${impacts || "无可安全解析的工件范围"}。${desktopRisk}；${cliRisk}。是否继续？`;
}

function artifactName(artifact: "config" | "credentials"): string {
  return artifact === "config" ? "config.toml" : "auth.json";
}

function canApplyProvider(snapshot: EnvironmentSnapshot): boolean {
  return snapshot.state !== "conflict" || snapshot.takeoverAvailable;
}

function environmentFailureMessage(messageId: string): string {
  const messages: Record<string, string> = {
    "environment.state_unavailable": "无法读取 Codex 环境状态。",
    "environment.provider_not_found": "所选供应商已不存在，请刷新后重试。",
    "environment.managed_conflict": "管理区块已被外部修改，当前操作已停止。",
    "environment.config_invalid": "config.toml 无法安全迁移。",
    "environment.credentials_invalid": "auth.json 无法安全保留字段。",
    "environment.restore_unavailable": "当前没有可安全恢复的最近配置。",
    "environment.restore_conflict": "受管工件已发生外部变化，恢复已停止。",
    "environment.backup_invalid": "最近一次配置备份不完整，无法安全恢复。",
    "environment.openai_login_required": "请先在 Codex 中完成 OpenAI 登录。",
    "environment.openai_login_unavailable": "无法确认 Codex 登录状态，已阻止切换。",
  };
  return messages[messageId] ?? providerFailureMessages[messageId] ?? "Codex 环境未发生变化，请重试。";
}

function AddressSuggestionDialog({
  requestedBaseUrl,
  suggestedBaseUrl,
  onAccept,
  onReject,
}: {
  requestedBaseUrl: string;
  suggestedBaseUrl: string;
  onAccept: () => void;
  onReject: () => void;
}) {
  return (
    <div className="dialog-backdrop">
      <section
        className="confirmation-dialog address-suggestion-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="address-suggestion-title"
      >
        <h2 id="address-suggestion-title">{providerMessages.addressSuggestionTitle}</h2>
        <p>{providerMessages.addressSuggestionMessage}</p>
        <dl className="address-comparison">
          <div>
            <dt>{providerMessages.requestedAddress}</dt>
            <dd>{requestedBaseUrl}</dd>
          </div>
          <div>
            <dt>{providerMessages.suggestedAddress}</dt>
            <dd>{suggestedBaseUrl}</dd>
          </div>
        </dl>
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onReject} autoFocus>
            {providerMessages.keepRequestedAddress}
          </button>
          <button className="command-button" type="button" onClick={onAccept}>
            {providerMessages.acceptSuggestedAddress}
          </button>
        </div>
      </section>
    </div>
  );
}

function ConfirmationDialog({
  title,
  message,
  primaryLabel,
  secondaryLabel,
  onPrimary,
  onSecondary,
  danger = false,
  primaryDisabled = false,
}: {
  title: string;
  message: string;
  primaryLabel: string;
  secondaryLabel: string;
  onPrimary: () => void;
  onSecondary: () => void;
  danger?: boolean;
  primaryDisabled?: boolean;
}) {
  return (
    <div className="dialog-backdrop">
      <section className="confirmation-dialog" role="dialog" aria-modal="true" aria-labelledby="dialog-title">
        <h2 id="dialog-title">{title}</h2>
        <p>{message}</p>
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onSecondary} autoFocus>
            {secondaryLabel}
          </button>
          <button
            className={danger ? "danger-button" : "command-button"}
            type="button"
            onClick={onPrimary}
            disabled={primaryDisabled}
          >
            {primaryLabel}
          </button>
        </div>
      </section>
    </div>
  );
}

function candidateStatus(
  operation: Operation,
  stage: ProviderValidationStage | "idle" | "complete",
  failure: ProviderFailure | null,
): string {
  if (failure) return providerMessages.candidateFailed;
  if (operation === "validating") return providerMessages.candidateVerifying;
  if (stage === "complete" || operation === "verified" || operation === "saving") {
    return providerMessages.candidateVerified;
  }
  return providerMessages.candidateUnverified;
}

function createValidationSession(source: ProviderValidationSource): ProviderValidationSession {
  return {
    source,
    status: "running",
    stage: "models_confirmed",
    stageStartedAt: Date.now(),
    failure: null,
  };
}

function formatVerifiedAt(epochSeconds: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hourCycle: "h23",
  }).format(new Date(epochSeconds * 1_000));
}

let requestSequence = 0;

function createRequestId(): string {
  requestSequence += 1;
  return `provider-request-${Date.now()}-${requestSequence}`;
}
