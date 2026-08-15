import { useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Eye,
  EyeOff,
  ExternalLink,
  GripVertical,
  LoaderCircle,
  LogIn,
  Pencil,
  Pin,
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
  onProviderSwitchRequested,
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
  applyWslProvider,
  asEnvironmentFailure,
  asWslFailure,
  getEnvironmentSnapshot,
  listWslEnvironments,
  restoreLastEnvironmentConfig,
  switchToOpenAiLogin,
  type EnvironmentFailure,
  type EnvironmentSnapshot,
  type WslEnvironmentSummary,
} from "./contracts/environment";
import {
  providerFailureMessages,
  providerMessages,
  restoreAvailabilityMessages,
  wslAvailabilityMessages,
  wslFailureMessages,
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
type ConfigChangeRequest =
  | { kind: "provider"; provider: ProviderSummary }
  | { kind: "openai" }
  | { kind: "provider_update"; validationId: string; provider: ProviderSummary; name: string };

const DAYWAY_NAME = "DayWay";
const DAYWAY_BASE_URL = "https://dayway.site/v1";

export default function ProviderPage() {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [listState, setListState] = useState<"loading" | "ready" | "error">("loading");
  const [environment, setEnvironment] = useState<EnvironmentSnapshot | null>(null);
  const [environmentState, setEnvironmentState] = useState<"loading" | "ready" | "error">("loading");
  const [environmentFailure, setEnvironmentFailure] = useState<EnvironmentFailure | null>(null);
  const [restoring, setRestoring] = useState(false);
  const [switchingMode, setSwitchingMode] = useState(false);
  const [switchingProviderId, setSwitchingProviderId] = useState<string | null>(null);
  const [view, setView] = useState<PageView>("catalog");
  const [confirmation, setConfirmation] = useState<Confirmation>(null);
  const [configChangeRequest, setConfigChangeRequest] = useState<ConfigChangeRequest | null>(null);
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
  const [wslEnvironments, setWslEnvironments] = useState<WslEnvironmentSummary[]>([]);
  const [wslState, setWslState] = useState<"loading" | "ready" | "error">("loading");
  const [wslDialogOpen, setWslDialogOpen] = useState(false);
  const [wslEnvironmentId, setWslEnvironmentId] = useState<string | null>(null);
  const [wslProviderId, setWslProviderId] = useState<string | null>(null);
  const [wslBusy, setWslBusy] = useState(false);
  const [wslFailure, setWslFailure] = useState<{ messageId: string } | null>(null);
  const [wslResult, setWslResult] = useState<{ provider: string; environment: string; pendingRestart: boolean } | null>(null);
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
    void listWslEnvironments()
      .then((items) => {
        if (mounted) {
          setWslEnvironments(items);
          setWslState("ready");
        }
      })
      .catch(() => {
        if (mounted) setWslState("error");
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
    void onProviderSwitchRequested(async (providerId) => {
      try {
        const [snapshot, catalog] = await Promise.all([
          getEnvironmentSnapshot(),
          listProviders(),
        ]);
        if (disposed) return;
        setEnvironment(snapshot);
        setProviders(catalog);
        const provider = catalog.find((item) => item.id === providerId);
        if (provider && !provider.isCurrent && canApplyProvider(snapshot)) {
          setConfigChangeRequest({ kind: "provider", provider });
        }
      } catch {
        if (!disposed) setEnvironmentState("error");
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
          const request: ConfigChangeRequest = {
            kind: "provider_update",
            validationId: receipt.validationId,
            provider: selected,
            name,
          };
          setConfigChangeRequest(request);
          setOperation("verified");
          return;
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
    setConfigChangeRequest({ kind: "provider", provider });
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
    setRestoring(true);
    setEnvironmentFailure(null);
    try {
      const updated = await restoreLastEnvironmentConfig(true, environment.revision);
      applyEnvironmentSnapshot(updated);
    } catch (error) {
      const refreshed = await refreshEnvironmentAfterFailure();
      const environmentFailure = asEnvironmentFailure(error);
      setEnvironmentFailure(refreshed ? environmentFailure : {
        ...environmentFailure,
        messageId: "environment.refresh_failed_after_change",
      });
    } finally {
      setRestoring(false);
    }
  }

  async function enableOpenAiLogin() {
    if (!environment || environment.loginStatus !== "logged_in" || environment.mode === "openai_login") return;
    setConfigChangeRequest({ kind: "openai" });
  }

  async function openWslDialog() {
    setWslDialogOpen(true);
    setWslFailure(null);
    setWslResult(null);
    setWslState("loading");
    try {
      const items = await listWslEnvironments();
      setWslEnvironments(items);
      setWslState("ready");
      const firstManageable = items.find((item) =>
        item.availability === "manageable" || item.availability === "default_user_changed");
      setWslEnvironmentId(firstManageable?.environmentId ?? items[0]?.environmentId ?? null);
      setWslProviderId(providers.find((provider) => !provider.isCurrent)?.id ?? providers[0]?.id ?? null);
    } catch (error) {
      setWslState("error");
      setWslFailure(asWslFailure(error));
    }
  }

  async function refreshWslDialog() {
    setWslFailure(null);
    setWslState("loading");
    try {
      const items = await listWslEnvironments();
      setWslEnvironments(items);
      setWslState("ready");
      setWslEnvironmentId((current) => current && items.some((item) => item.environmentId === current)
        ? current
        : items.find((item) =>
          item.availability === "manageable" || item.availability === "default_user_changed")?.environmentId
          ?? items[0]?.environmentId
          ?? null);
    } catch (error) {
      setWslState("error");
      setWslFailure(asWslFailure(error));
    }
  }

  async function applyWslSelection() {
    const target = wslEnvironments.find((item) => item.environmentId === wslEnvironmentId);
    if (
      !target ||
      !wslProviderId ||
      (target.availability !== "manageable" && target.availability !== "default_user_changed")
    ) return;
    const provider = providers.find((item) => item.id === wslProviderId);
    if (!provider) return;
    setWslBusy(true);
    setWslFailure(null);
    setWslResult(null);
    try {
      const result = await applyWslProvider(target.environmentId, provider.id, target.revision, true);
      setWslEnvironments((current) => current.map((item) => item.environmentId === result.environment.environmentId
        ? result.environment
        : item));
      setWslResult({
        provider: provider.name,
        environment: target.displayName,
        pendingRestart: result.pendingRestart,
      });
      setCatalogFeedback(result.pendingRestart
        ? `${providerMessages.wslPendingRestart} ${providerMessages.wslApplied(provider.name, target.displayName)}`
        : providerMessages.wslApplied(provider.name, target.displayName));
    } catch (error) {
      const failure = asWslFailure(error);
      await refreshWslDialog();
      setWslFailure(failure);
    } finally {
      setWslBusy(false);
    }
  }

  async function executeConfigChange(request: ConfigChangeRequest) {
    if (!environment) return;
    setConfigChangeRequest(null);
    if (request.kind === "provider") setSwitchingProviderId(request.provider.id);
    else if (request.kind === "openai") setSwitchingMode(true);
    else setOperation("saving");
    setEnvironmentFailure(null);
    try {
      let updated: EnvironmentSnapshot;
      if (request.kind === "provider") {
        updated = await applyEnvironmentProvider(
          request.provider.id,
          environment.revision,
        );
      } else if (request.kind === "openai") {
        updated = await switchToOpenAiLogin(environment.revision);
      } else {
        const result = await saveAndApplyProviderUpdate(
          request.validationId,
          request.provider.id,
          request.name,
        );
        updated = result.environment;
        receiptRef.current = null;
        replaceProvider(result.provider);
        resetEditor("catalog");
      }
      applyEnvironmentSnapshot(updated);
      if (updated.pendingRestart) {
        setCatalogFeedback(providerMessages.configChangePendingRestart);
      }
    } catch (error) {
      const refreshed = await refreshEnvironmentAfterFailure();
      if (request.kind === "provider_update") {
        const providerFailure = asProviderFailure(error);
        setFailure(refreshed ? providerFailure : {
          ...providerFailure,
          messageId: "environment.refresh_failed_after_change",
        });
      } else {
        const environmentFailure = asEnvironmentFailure(error);
        setEnvironmentFailure(refreshed ? environmentFailure : {
          ...environmentFailure,
          messageId: "environment.refresh_failed_after_change",
        });
      }
    } finally {
      if (request.kind === "provider") setSwitchingProviderId(null);
      else if (request.kind === "openai") setSwitchingMode(false);
      else setOperation("idle");
    }
  }

  async function refreshEnvironmentAfterFailure(): Promise<boolean> {
    try {
      applyEnvironmentSnapshot(await getEnvironmentSnapshot());
      setEnvironmentState("ready");
      return true;
    } catch {
      setEnvironment(null);
      setProviders((current) => current.map((provider) => ({
        ...provider,
        isCurrent: false,
      })));
      setEnvironmentState("error");
      return false;
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
        <h1>{providerMessages.pageTitle}</h1>
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
        <EnvironmentReadNotice state={environmentState} />
        <section className="provider-catalog" aria-labelledby="provider-catalog-heading">
          <div className="catalog-heading">
            <h2 id="provider-catalog-heading">{providerMessages.catalogTitle}</h2>
            <span>{providerMessages.catalogCount(providers.length)}</span>
          </div>
          {listState === "loading" && <p className="pane-note">{providerMessages.loadingCatalog}</p>}
          {listState === "error" && <p className="inline-error">{providerMessages.catalogUnavailable}</p>}
          <div className="provider-list" aria-label={providerMessages.verifiedProviders}>
            {!savedDayway && (
              <article className="provider-list-row provider-template-row">
                <span
                  className="provider-fixed-indicator"
                  role="img"
                  aria-label={providerMessages.fixedProviderAccessibleName(DAYWAY_NAME)}
                  title={providerMessages.fixedProviderAccessibleName(DAYWAY_NAME)}
                >
                  <Pin size={16} aria-hidden="true" />
                </span>
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
                  <DaywayWebsiteButton onVisit={visitDaywayWebsite} />
                  <button className="command-button compact" type="button" onClick={configureDayway} disabled={busy} aria-label={providerMessages.configureDayway}>
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
                  <span
                    className="provider-fixed-indicator"
                    role="img"
                    aria-label={providerMessages.fixedProviderAccessibleName(provider.name)}
                    title={providerMessages.fixedProviderAccessibleName(provider.name)}
                  >
                    <Pin size={16} aria-hidden="true" />
                  </span>
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
                  </div>
                  <span className="provider-row-url" title={provider.baseUrl}>{provider.baseUrl}</span>
                  <span className="provider-row-model" title={provider.defaultModel}>
                    {provider.defaultModel}
                  </span>
                </div>
                <div className="provider-row-actions">
                  {provider.recommendationId === "dayway" && (
                    <DaywayWebsiteButton onVisit={visitDaywayWebsite} />
                  )}
                  <button
                    className="secondary-button compact row-icon-button"
                    type="button"
                    onClick={() => void runRevalidation(provider)}
                    disabled={busy}
                    aria-label={providerMessages.verifyProviderAccessibleName(provider.name)}
                    title={providerMessages.verify}
                  >
                    <RefreshCw size={16} aria-hidden="true" />
                  </button>
                  <button
                    className="secondary-button compact row-icon-button"
                    type="button"
                    onClick={() => editProvider(provider)}
                    disabled={busy}
                    aria-label={providerMessages.editProviderAccessibleName(provider.name)}
                    title={providerMessages.editProvider}
                  >
                    <Pencil size={16} aria-hidden="true" />
                  </button>
                  <button
                    className="danger-button compact row-icon-button"
                    type="button"
                    onClick={() => void deleteCatalogProvider(provider)}
                    disabled={provider.isCurrent || busy}
                    aria-label={providerMessages.deleteProviderAccessibleName(provider.name)}
                    title={providerMessages.deleteProvider}
                  >
                    <Trash2 size={16} aria-hidden="true" />
                  </button>
                  <button
                    className="command-button compact"
                    type="button"
                    onClick={() => void switchCatalogProvider(provider)}
                    disabled={provider.isCurrent || busy || switchingProviderId === provider.id || !environment || !canApplyProvider(environment)}
                    aria-label={provider.isCurrent
                      ? providerMessages.currentProviderAccessibleName(provider.name)
                      : providerMessages.switchProviderAccessibleName(provider.name)}
                  >
                    {switchingProviderId === provider.id
                      ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                      : <Check size={16} aria-hidden="true" />}
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
        <WslActions
          environments={wslEnvironments}
          state={wslState}
          busy={busy || wslBusy}
          onOpen={() => void openWslDialog()}
        />
        <EnvironmentActions
          snapshot={environment}
          failure={environmentFailure}
          busy={busy || wslBusy}
          restoring={restoring}
          switchingMode={switchingMode}
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
      {configChangeRequest && (
        <ConfirmationDialog
          title={providerMessages.configChangeTitle}
          message={configChangeMessage(configChangeRequest)}
          primaryLabel={providerMessages.configChangePrimary}
          secondaryLabel={providerMessages.configChangeCancel}
          onPrimary={() => void executeConfigChange(configChangeRequest)}
          onSecondary={() => setConfigChangeRequest(null)}
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
      {wslDialogOpen && (
        <WslProviderDialog
          environments={wslEnvironments}
          providers={providers}
          state={wslState}
          selectedEnvironmentId={wslEnvironmentId}
          selectedProviderId={wslProviderId}
          busy={wslBusy}
          failure={wslFailure}
          result={wslResult}
          onEnvironmentChange={setWslEnvironmentId}
          onProviderChange={setWslProviderId}
          onRefresh={() => void refreshWslDialog()}
          onApply={() => void applyWslSelection()}
          onClose={() => {
            if (!wslBusy) setWslDialogOpen(false);
          }}
        />
      )}
    </>
  );
}

function EnvironmentReadNotice({
  state,
}: {
  state: "loading" | "ready" | "error";
}) {
  if (state === "loading") {
    return <p className="environment-status-note">{providerMessages.environmentReading}</p>;
  }
  if (state === "error") {
    return <p className="environment-status-note is-error" role="alert">{providerMessages.environmentReadFailed}</p>;
  }
  return null;
}

function DaywayWebsiteButton({ onVisit }: { onVisit: () => Promise<void> }) {
  return (
    <button
      className="secondary-button compact row-icon-button"
      type="button"
      onClick={() => void onVisit()}
      aria-label={providerMessages.visitDaywayWebsiteAccessibleName}
      title={providerMessages.visitDaywayWebsite}
    >
      <ExternalLink size={16} aria-hidden="true" />
    </button>
  );
}

function WslActions({
  environments,
  state,
  busy,
  onOpen,
}: {
  environments: WslEnvironmentSummary[];
  state: "loading" | "ready" | "error";
  busy: boolean;
  onOpen: () => void;
}) {
  const manageable = environments.some((environment) =>
    environment.availability === "manageable" || environment.availability === "default_user_changed");
  const description = state === "loading"
    ? "正在读取 WSL2 发行版状态。"
    : state === "error"
      ? providerMessages.wslReadFailed
      : manageable
        ? "选择一个 WSL2 发行版和已验证供应商。"
        : providerMessages.wslNoManageable;
  return (
    <section className="environment-tools environment-tools-group" aria-label={providerMessages.otherEnvironmentActions}>
      <div className="environment-tools-heading">{providerMessages.otherEnvironmentActions}</div>
      <button
        className="secondary-button wsl-environment-command"
        type="button"
        onClick={onOpen}
        disabled={busy || state === "loading"}
        aria-description={description}
      >
        <Server size={17} aria-hidden="true" />
        {providerMessages.chooseWslProvider}
      </button>
      <button className="secondary-button upcoming-command" type="button" disabled>
        {providerMessages.exportLinuxScript} <span>{providerMessages.comingSoon}</span>
      </button>
    </section>
  );
}

function WslProviderDialog({
  environments,
  providers,
  state,
  selectedEnvironmentId,
  selectedProviderId,
  busy,
  failure,
  result,
  onEnvironmentChange,
  onProviderChange,
  onRefresh,
  onApply,
  onClose,
}: {
  environments: WslEnvironmentSummary[];
  providers: ProviderSummary[];
  state: "loading" | "ready" | "error";
  selectedEnvironmentId: string | null;
  selectedProviderId: string | null;
  busy: boolean;
  failure: { messageId: string } | null;
  result: { provider: string; environment: string; pendingRestart: boolean } | null;
  onEnvironmentChange: (id: string) => void;
  onProviderChange: (id: string) => void;
  onRefresh: () => void;
  onApply: () => void;
  onClose: () => void;
}) {
  const selectedEnvironment = environments.find((item) => item.environmentId === selectedEnvironmentId) ?? null;
  const manageable = selectedEnvironment?.availability === "manageable" || selectedEnvironment?.availability === "default_user_changed";
  const canApply = state === "ready" && manageable && Boolean(selectedProviderId) && !busy;
  const failureMessage = failure ? wslFailureMessages[failure.messageId] ?? wslFailureMessages["wsl.state_unavailable"] : "";
  return (
    <div className="dialog-backdrop">
      <section className="confirmation-dialog wsl-provider-dialog" role="dialog" aria-modal="true" aria-labelledby="wsl-dialog-title">
        <div className="dialog-heading">
          <div>
            <h2 id="wsl-dialog-title">{providerMessages.wslDialogTitle}</h2>
            <p>{providerMessages.wslDialogSubtitle}</p>
          </div>
          <button className="field-icon-button" type="button" onClick={onClose} disabled={busy} aria-label={providerMessages.wslCancel}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        {state === "loading" && <p className="pane-note">{providerMessages.environmentReading}</p>}
        {state === "error" && (
          <div className="wsl-dialog-error">
            <p className="inline-error" role="alert">{failureMessage || providerMessages.wslReadFailed}</p>
            <button className="secondary-button" type="button" onClick={onRefresh} disabled={busy}>
              <RefreshCw size={16} aria-hidden="true" />
              {providerMessages.wslRefresh}
            </button>
          </div>
        )}
        {state === "ready" && (
          <>
            <div className="wsl-dialog-fields">
              <label className="form-field">
                <span>{providerMessages.wslDistribution}</span>
                <select value={selectedEnvironmentId ?? ""} onChange={(event) => onEnvironmentChange(event.target.value)} disabled={busy}>
                  <option value="">{providerMessages.wslChooseDistribution}</option>
                  {environments.map((environment) => (
                    <option
                      key={environment.environmentId}
                      value={environment.environmentId}
                      disabled={environment.availability !== "manageable" && environment.availability !== "default_user_changed"}
                    >
                      {environment.displayName} · {wslAvailabilityMessages[environment.availability]}
                    </option>
                  ))}
                </select>
              </label>
              <label className="form-field">
                <span>{providerMessages.wslProvider}</span>
                <select value={selectedProviderId ?? ""} onChange={(event) => onProviderChange(event.target.value)} disabled={busy || providers.length === 0}>
                  <option value="">{providerMessages.wslChooseProvider}</option>
                  {providers.map((provider) => <option key={provider.id} value={provider.id}>{provider.name}</option>)}
                </select>
              </label>
            </div>
            {selectedEnvironment && (
              <div className="wsl-environment-detail">
                <div className="provider-row-title">
                  <strong>{selectedEnvironment.displayName}</strong>
                  <span className={selectedEnvironment.running ? "current-badge" : "verified-badge"}>
                    {selectedEnvironment.running ? providerMessages.wslRunning : providerMessages.wslStopped}
                  </span>
                  <span className="pending-badge">{wslAvailabilityMessages[selectedEnvironment.availability]}</span>
                </div>
                {selectedEnvironment.currentProvider && <p>当前供应商：{selectedEnvironment.currentProvider.name}</p>}
                {selectedEnvironment.defaultUid !== null && <p>默认用户 UID：{selectedEnvironment.defaultUid}</p>}
                {selectedEnvironment.running
                  ? <p>{providerMessages.wslRunningWarning}</p>
                  : <p>{providerMessages.wslStoppedWarning}</p>}
                <p>{providerMessages.wslDefaultUserScope}</p>
              </div>
            )}
            {result && (
              <p className="catalog-feedback" role="status">
                {providerMessages.wslApplied(result.provider, result.environment)} {result.pendingRestart ? providerMessages.wslPendingRestart : ""}
              </p>
            )}
            {failure && <p className="inline-error" role="alert">{failureMessage}</p>}
          </>
        )}
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onClose} disabled={busy}>{providerMessages.wslCancel}</button>
          <button className="command-button" type="button" onClick={onApply} disabled={!canApply}>
            {busy ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}
            {busy ? providerMessages.wslApplying : providerMessages.wslApply}
          </button>
        </div>
      </section>
    </div>
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
    ? providerMessages.environmentUnavailable
    : snapshot.mode === "openai_login"
      ? snapshot.loginStatus === "not_logged_in"
        ? providerMessages.openAiLoginExpired
        : snapshot.loginStatus === "unavailable"
          ? providerMessages.openAiLoginUnconfirmed
          : providerMessages.alreadyOpenAiLogin
      : snapshot.loginStatus === "not_logged_in"
        ? providerMessages.openAiLoginRequired
        : snapshot.loginStatus === "unavailable"
          ? providerMessages.openAiLoginBlocked
          : providerMessages.openAiLoginAvailable;
  return (
    <section className="environment-tools environment-tools-group" aria-label={providerMessages.windowsEnvironmentActions}>
      <div className="environment-tools-heading">{providerMessages.windowsEnvironmentActions}</div>
      <button
        className="secondary-button environment-command"
        type="button"
        onClick={onRestore}
        disabled={busy || restoring || restoreAvailability !== "available"}
        aria-description={restoring ? providerMessages.restoringConfiguration : restoreAvailabilityMessages[restoreAvailability]}
      >
        {restoring ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <RotateCcw size={17} aria-hidden="true" />}
        {providerMessages.restoreConfiguration}
      </button>
      <button
        className="secondary-button environment-command"
        type="button"
        onClick={onSwitchMode}
        disabled={busy || switchingMode || !snapshot || snapshot.mode === "openai_login" || snapshot.loginStatus !== "logged_in"}
        aria-description={openAiReason}
      >
        {switchingMode ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <LogIn size={17} aria-hidden="true" />}
        {providerMessages.switchToOpenAiLogin}
      </button>
      {failure && <p className="inline-error environment-tool-error" role="alert">{environmentFailureMessage(failure.messageId)}</p>}
    </section>
  );
}

function configChangeMessage(request: ConfigChangeRequest): string {
  const risk = providerMessages.configChangeConsumerRisk;
  if (request.kind === "provider") {
    return `${providerMessages.configChangeProviderTarget(request.provider.name)}${risk}`;
  }
  if (request.kind === "provider_update") {
    return `${providerMessages.configChangeProviderUpdateTarget(request.provider.name)}${risk}`;
  }
  return `${providerMessages.configChangeOpenAiTarget}${risk}`;
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

let requestSequence = 0;

function createRequestId(): string {
  requestSequence += 1;
  return `provider-request-${Date.now()}-${requestSequence}`;
}
