import { useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Download,
  Eye,
  EyeOff,
  ExternalLink,
  GripVertical,
  LoaderCircle,
  Pencil,
  Pin,
  Plus,
  RefreshCw,
  Server,
  ShieldAlert,
  ShieldCheck,
  Trash2,
  X,
} from "lucide-react";

import ProviderValidationDialog, {
  type ProviderValidationSession,
  type ProviderValidationSource,
} from "./ProviderValidationDialog";
import type { OpenAiSidebarAction } from "./AppSidebar";

import {
  asProviderFailure,
  asLinuxExportFailure,
  cancelProviderRequest,
  confirmProviderValidationBaseUrl,
  copyProviderApiKey,
  chooseLinuxExportDestination,
  deleteProvider,
  discoverProviderModels,
  discoverProviderModelsForUpdate,
  discardProviderValidation,
  exportLinuxScript,
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
  type DeleteProviderResult,
  type ProviderFailure,
  type ProviderSummary,
  type ProviderValidationReceipt,
  type ProviderValidationStage,
  type LinuxExportFailure,
  type LinuxExportResult,
  type LinuxShell,
} from "./contracts/provider";
import {
  applyEnvironmentProvider,
  applyWslProvider,
  asEnvironmentFailure,
  asWslFailure,
  getEnvironmentSnapshot,
  listWslEnvironments,
  refreshWslEnvironment,
  switchToOpenAiLogin,
  type EnvironmentFailure,
  type EnvironmentSnapshot,
  type WslEnvironmentSummary,
  type WslLifecycleOutcome,
  type WslLifecycleResult,
} from "./contracts/environment";
import {
  providerFailureMessages,
  providerMessages,
  wslAvailabilityMessages,
  wslConfigurationStateMessages,
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

const linuxShellPresentation: Record<LinuxShell, {
  displayName: string;
  titleName: string;
  startupFile: string;
  suggestedFileName: string;
}> = {
  bash: {
    displayName: "Bash 4+",
    titleName: "Bash",
    startupFile: ".bashrc",
    suggestedFileName: "gpteasy.sh",
  },
  zsh: {
    displayName: "Zsh 5+",
    titleName: "Zsh",
    startupFile: ".zshrc",
    suggestedFileName: "gpteasy.zsh",
  },
};

type PageView = "catalog" | "detail";
type Confirmation = "discard" | "validation" | null;
type LinuxExportStep = "shell" | "success" | null;
type ConfigChangeRequest =
  | { kind: "provider"; provider: ProviderSummary; firstSaved?: boolean }
  | { kind: "openai" }
  | { kind: "provider_update"; validationId: string; provider: ProviderSummary; name: string };

const DAYWAY_NAME = "DayWay";
const DAYWAY_BASE_URL = "https://dayway.site/v1";

function isManageableWslEnvironment(environment: WslEnvironmentSummary): boolean {
  return environment.availability === "manageable"
    || environment.availability === "default_user_changed";
}

function canApplyWslProvider(environment: WslEnvironmentSummary): boolean {
  return isManageableWslEnvironment(environment)
    && environment.configurationState !== "conflict"
    && environment.configurationState !== "busy";
}

export default function ProviderPage({
  onOpenAiActionChange,
  onCurrentProviderNameChange,
}: {
  onOpenAiActionChange?: (action: OpenAiSidebarAction) => void;
  onCurrentProviderNameChange?: (name: string | null) => void;
  onOpenSessions?: () => void;
}) {
  const [providers, setProviders] = useState<ProviderSummary[]>([]);
  const [listState, setListState] = useState<"loading" | "ready" | "error">("loading");
  const [environment, setEnvironment] = useState<EnvironmentSnapshot | null>(null);
  const [environmentState, setEnvironmentState] = useState<"loading" | "ready" | "error">("loading");
  const [environmentFailure, setEnvironmentFailure] = useState<EnvironmentFailure | null>(null);
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
  const [wslAction, setWslAction] = useState<"idle" | "applying" | "refreshing">("idle");
  const [wslFailure, setWslFailure] = useState<{ messageId: string } | null>(null);
  const [wslFeedback, setWslFeedback] = useState("");
  const [linuxExportStep, setLinuxExportStep] = useState<LinuxExportStep>(null);
  const [linuxExportShell, setLinuxExportShell] = useState<LinuxShell>("bash");
  const [linuxExportResult, setLinuxExportResult] = useState<LinuxExportResult | null>(null);
  const [linuxExportFailure, setLinuxExportFailure] = useState<LinuxExportFailure | null>(null);
  const [linuxExportBusy, setLinuxExportBusy] = useState(false);
  const [copyStatus, setCopyStatus] = useState("");
  const apiKeyRef = useRef<HTMLInputElement | null>(null);
  const activeRequest = useRef<string | null>(null);
  const draggedProviderId = useRef<string | null>(null);
  const receiptRef = useRef<string | null>(null);
  const saveAfterValidation = useRef(false);
  const selected = providers.find((provider) => provider.id === selectedId) ?? null;
  const savedDayway = providers.find((provider) => provider.recommendationId === "dayway") ?? null;
  const isDaywayEditor = isRecommendedCandidate || selected?.recommendationId === "dayway";
  const wslBusy = wslAction !== "idle";

  useEffect(() => {
    let mounted = true;
    void listProviders()
      .then((items) => {
        if (mounted) {
          setProviders(items);
          setWslProviderId((current) => current && items.some((provider) => provider.id === current)
            ? current
            : items.find((provider) => !provider.isCurrent)?.id ?? items[0]?.id ?? null);
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
    if (!environment || listState !== "ready") return;
    const currentProviderId = environment.currentProvider?.id ?? null;
    setProviders((current) => {
      if (current.every((provider) => provider.isCurrent === (provider.id === currentProviderId))) {
        return current;
      }
      return current.map((provider) => ({
        ...provider,
        isCurrent: provider.id === currentProviderId,
      }));
    });
  }, [environment, listState]);

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

  function discardReceipt(
    nextStage: ProviderValidationStage | "idle" | "complete" = "idle",
  ) {
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
    discardReceipt("complete");
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

  async function runValidation(continueSave = false) {
    saveAfterValidation.current = continueSave;
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
      saveAfterValidation.current = false;
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
    const shouldContinueSave = validationSession.source.kind === "detail"
      && validationSession.status === "succeeded"
      && saveAfterValidation.current;
    if (validationSession.source.kind === "catalog") {
      setCatalogFeedback(validationSession.status === "succeeded"
        ? `${validationSession.source.providerName} 重新验证成功。`
        : `${validationSession.source.providerName} 最近验证失败。`);
      setFailure(null);
    }
    setValidationSession(null);
    if (shouldContinueSave && !addressSuggestion && receipt) {
      saveAfterValidation.current = false;
      void saveProvider(receipt);
    }
  }

  async function acceptAddressSuggestion() {
    if (!addressSuggestion) return;
    const suggestion = addressSuggestion;
    try {
      await confirmProviderValidationBaseUrl(
        suggestion.validationId,
        suggestion.normalizedBaseUrl,
      );
      setBaseUrl(suggestion.normalizedBaseUrl);
      setAddressSuggestion(null);
      if (saveAfterValidation.current) {
        saveAfterValidation.current = false;
        await saveProvider(suggestion);
      }
    } catch (error) {
      saveAfterValidation.current = false;
      discardReceipt("models_confirmed");
      setFailure(asProviderFailure(error));
    }
  }

  function rejectAddressSuggestion() {
    saveAfterValidation.current = false;
    discardReceipt("models_confirmed");
  }

  async function saveProvider(validatedReceipt = receipt) {
    if ((!selected || criticalDirty) && !validatedReceipt) {
      setConfirmation("validation");
      return;
    }
    saveAfterValidation.current = false;
    setOperation("saving");
    setFailure(null);
    try {
      let saved: ProviderSummary;
      if (!selected) {
        if (!validatedReceipt) return;
        const isFirstProvider = providers.length === 0;
        if (isRecommendedCandidate) {
          try {
            saved = await saveDaywayProvider(validatedReceipt.validationId);
          } catch (error) {
            const providerFailure = asProviderFailure(error);
            if (providerFailure.messageId !== "provider.recommended_name_conflict") throw error;
            if (!window.confirm(providerMessages.daywayNameConflictConfirmation)) {
              setOperation("verified");
              return;
            }
            saved = await saveDaywayProvider(validatedReceipt.validationId, true);
          }
        } else {
          saved = await saveVerifiedProvider(validatedReceipt.validationId, name);
        }
        receiptRef.current = null;
        setProviders((current) => isRecommendedCandidate ? [saved, ...current] : [...current, saved]);
        resetEditor("catalog");
        if (isFirstProvider && environment && canApplyProvider(environment)) {
          setConfigChangeRequest({ kind: "provider", provider: saved, firstSaved: true });
        }
        return;
      }
      if (criticalDirty) {
        if (!validatedReceipt) return;
        if (selected.isCurrent) {
          const request: ConfigChangeRequest = {
            kind: "provider_update",
            validationId: validatedReceipt.validationId,
            provider: selected,
            name,
          };
          setConfigChangeRequest(request);
          setOperation("verified");
          return;
        } else {
          saved = await saveProviderUpdate(validatedReceipt.validationId, selected.id, name);
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
      setOperation(validatedReceipt ? "verified" : "idle");
    }
  }

  async function deleteCatalogProvider(provider: ProviderSummary) {
    if (provider.isCurrent || !window.confirm(providerMessages.deleteConfirmation)) {
      return;
    }
    const hasStoppedWsl = wslEnvironments.some((environment) =>
      isManageableWslEnvironment(environment) && !environment.running);
    let authorizeStoppedWsl = false;
    if (hasStoppedWsl) {
      authorizeStoppedWsl = window.confirm(providerMessages.deleteWslVerificationConfirmation);
      if (!authorizeStoppedWsl) return;
    }
    setOperation("deleting");
    setFailure(null);
    try {
      let result: DeleteProviderResult;
      try {
        result = await deleteProvider(provider.id, authorizeStoppedWsl);
      } catch (error) {
        const failure = asProviderFailure(error);
        if (
          authorizeStoppedWsl
          || failure.messageId !== "wsl.delete_start_authorization_required"
          || !window.confirm(providerMessages.deleteWslVerificationConfirmation)
        ) {
          throw error;
        }
        authorizeStoppedWsl = true;
        result = await deleteProvider(provider.id, true);
      }
      setProviders((current) => current.filter((item) => item.id !== provider.id));
      if (selectedId === provider.id) resetEditor("catalog");
      setCatalogFeedback([
        providerMessages.providerDeleted,
        lifecycleResultsMessage(result.lifecycleResults),
      ].filter(Boolean).join(" "));
      setOperation("idle");
    } catch (error) {
      const failure = asProviderFailure(error);
      const lifecycleFeedback = failure.lifecycleResults?.length
        ? lifecycleResultsMessage(failure.lifecycleResults)
        : lifecycleOutcomeMessage(failure.lifecycleOutcome);
      setFailure(failure);
      setCatalogFeedback([
        providerFailureMessages[failure.messageId] ?? providerMessages.validationFallback,
        lifecycleFeedback,
      ].filter(Boolean).join(" "));
      setOperation("idle");
    }
  }

  async function switchCatalogProvider(provider: ProviderSummary) {
    if (provider.isCurrent || !environment || !canApplyProvider(environment)) return;
    setConfigChangeRequest({ kind: "provider", provider });
  }

  async function openWslDialog() {
    setWslDialogOpen(true);
    setWslFailure(null);
    setWslFeedback("");
    setWslState("loading");
    try {
      const items = await listWslEnvironments();
      setWslEnvironments(items);
      setWslState("ready");
      const firstManageable = items.find(isManageableWslEnvironment);
      setWslEnvironmentId(firstManageable?.environmentId ?? null);
      setWslProviderId(wslTargetProviderId(firstManageable, providers));
    } catch (error) {
      setWslState("error");
      setWslFailure(asWslFailure(error));
    }
  }

  async function refreshWslDialog() {
    setWslFailure(null);
    setWslFeedback("");
    setWslState("loading");
    try {
      const items = await listWslEnvironments();
      setWslEnvironments(items);
      setWslState("ready");
      const selectedEnvironment = items.find((item) =>
        item.environmentId === wslEnvironmentId && isManageableWslEnvironment(item))
        ?? items.find(isManageableWslEnvironment);
      setWslEnvironmentId(selectedEnvironment?.environmentId ?? null);
      setWslProviderId(wslTargetProviderId(selectedEnvironment, providers));
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
      !canApplyWslProvider(target)
    ) return;
    const provider = providers.find((item) => item.id === wslProviderId);
    if (!provider) return;
    setWslAction("applying");
    setWslFailure(null);
    setWslFeedback("");
    try {
      const result = await applyWslProvider(target.environmentId, provider.id, target.revision, true);
      setWslEnvironments((current) => current.map((item) => item.environmentId === result.environment.environmentId
        ? result.environment
        : item));
      setCatalogFeedback([
        providerMessages.wslApplied(provider.name, target.displayName),
        result.pendingRestart ? providerMessages.wslPendingRestart : "",
        lifecycleOutcomeMessage(result.lifecycleOutcome),
      ].filter(Boolean).join(" "));
      setWslDialogOpen(false);
    } catch (error) {
      const failure = asWslFailure(error);
      await refreshWslDialog();
      setWslFailure(failure);
    } finally {
      setWslAction("idle");
    }
  }

  async function refreshSelectedWslActualState() {
    const target = wslEnvironments.find((item) => item.environmentId === wslEnvironmentId);
    if (!target || !isManageableWslEnvironment(target)) return;
    const authorizeStart = target.running
      || window.confirm(providerMessages.wslStoppedWarning);
    if (!authorizeStart) return;
    setWslAction("refreshing");
    setWslFailure(null);
    setWslFeedback("");
    try {
      const result = await refreshWslEnvironment(
        target.environmentId,
        target.revision,
        authorizeStart,
      );
      setWslEnvironments((current) => current.map((item) =>
        item.environmentId === result.environment.environmentId ? result.environment : item));
      setWslProviderId(wslTargetProviderId(result.environment, providers));
      setWslFeedback([
        providerMessages.wslRefreshed(target.displayName),
        lifecycleOutcomeMessage(result.lifecycleOutcome),
      ].filter(Boolean).join(" "));
    } catch (error) {
      const failure = asWslFailure(error);
      setWslFailure(failure);
      if (failure.lifecycleOutcome) {
        setWslFeedback(lifecycleOutcomeMessage(failure.lifecycleOutcome));
      }
    } finally {
      setWslAction("idle");
    }
  }

  function openLinuxExport() {
    if (providers.length === 0) return;
    setLinuxExportShell("bash");
    setLinuxExportResult(null);
    setLinuxExportFailure(null);
    setLinuxExportStep("shell");
  }

  function closeLinuxExport() {
    if (linuxExportBusy) return;
    setLinuxExportStep(null);
    setLinuxExportResult(null);
    setLinuxExportFailure(null);
  }

  async function chooseLinuxExportPath() {
    setLinuxExportBusy(true);
    setLinuxExportFailure(null);
    try {
      const selected = await chooseLinuxExportDestination(linuxExportShell);
      if (!selected) {
        setLinuxExportStep(null);
        setLinuxExportResult(null);
        setLinuxExportFailure(null);
        return;
      }
      await performLinuxExport(selected.path, selected.exists);
    } catch (error) {
      setLinuxExportFailure(asLinuxExportFailure(error));
    } finally {
      setLinuxExportBusy(false);
    }
  }

  async function performLinuxExport(destination: string, confirmOverwrite: boolean) {
    setLinuxExportBusy(true);
    setLinuxExportFailure(null);
    try {
      const result = await exportLinuxScript(linuxExportShell, destination, confirmOverwrite);
      setLinuxExportResult(result);
      setLinuxExportStep("success");
    } catch (error) {
      setLinuxExportFailure(asLinuxExportFailure(error));
    } finally {
      setLinuxExportBusy(false);
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
  const savedCombinationUnchanged = selected !== null && !criticalDirty;
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
    !busy;
  const canSave = name.trim().length > 0 && !busy && (selected ? dirty : true);
  const errorId = failure ? "provider-validation-error" : undefined;
  const openAiReason = openAiLoginReason(environment);
  const openAiCurrent = environment?.mode === "openai_login";
  const openAiDisabled = busy
    || wslBusy
    || switchingMode
    || !environment
    || environment.loginStatus !== "logged_in";

  useEffect(() => {
    onOpenAiActionChange?.({
      busy: switchingMode,
      current: openAiCurrent,
      description: openAiReason,
      disabled: openAiDisabled,
      onSelect: () => {
        if (!environment || environment.loginStatus !== "logged_in" || openAiCurrent) return;
        setConfigChangeRequest({ kind: "openai" });
      },
    });
  }, [environment, onOpenAiActionChange, openAiCurrent, openAiDisabled, openAiReason, switchingMode]);

  useEffect(() => {
    onCurrentProviderNameChange?.(openAiCurrent ? "OpenAI 登录模式" : environment?.currentProvider?.name ?? null);
  }, [environment, onCurrentProviderNameChange, openAiCurrent]);

  return (
    <main className="main-content">
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
                    <RecommendedBadge onVisit={visitDaywayWebsite} />
                    <span className="pending-badge">待配置</span>
                  </div>
                  <span className="provider-row-url" title={DAYWAY_BASE_URL}>{DAYWAY_BASE_URL}</span>
                  <span className="provider-row-model">尚未选择</span>
                </div>
                <div className="provider-row-actions">
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
                    {provider.recommendationId === "dayway" && (
                      <RecommendedBadge onVisit={visitDaywayWebsite} />
                    )}
                    {provider.hasRecommendationUpdate && <span className="pending-badge">推荐地址已更新</span>}
                    <span className="verified-badge">{providerMessages.verified}</span>
                  </div>
                  <span className="provider-row-url" title={provider.baseUrl}>{provider.baseUrl}</span>
                  <span className="provider-row-model" title={provider.defaultModel}>
                    {provider.defaultModel}
                  </span>
                </div>
                <div className="provider-row-actions">
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
                      : providerMessages.selectProviderAccessibleName(provider.name)}
                  >
                    {switchingProviderId === provider.id
                      ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                      : <Check size={16} aria-hidden="true" />}
                    {provider.isCurrent ? providerMessages.currentProvider : providerMessages.selectProvider}
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
        <section className="environment-tools" aria-label={providerMessages.environmentActions}>
          <WslActions
            environments={wslEnvironments}
            providers={providers}
            state={wslState}
            busy={busy || wslBusy || linuxExportBusy}
            onOpen={() => void openWslDialog()}
            onExport={openLinuxExport}
          />
          {environmentFailure && (
            <p className="inline-error environment-tool-error" role="alert">
              {environmentFailureMessage(environmentFailure.messageId)}
            </p>
          )}
        </section>
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
              <div className="api-key-label-row">
                <label htmlFor="provider-api-key">{providerMessages.apiKey}</label>
                {isDaywayEditor && (
                  <span className="dayway-api-key-hint" role="note">
                    <span>{providerMessages.daywayApiKeyHint}</span>
                    <button
                      className="dayway-api-key-link"
                      type="button"
                      onClick={() => void visitDaywayWebsite()}
                      disabled={busy}
                    >
                      {providerMessages.daywayApiKeyLink}
                      <ExternalLink size={14} aria-hidden="true" />
                    </button>
                  </span>
                )}
              </div>
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
                  onClick={() => void (savedCombinationUnchanged
                    ? runRevalidation()
                    : runValidation())}
                  disabled={!canValidate}
                >
                  <ShieldCheck size={17} aria-hidden="true" />
                  {savedCombinationUnchanged
                    ? providerMessages.revalidate
                    : selected
                      ? providerMessages.validateUpdate
                      : providerMessages.validateProvider}
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
            void runValidation(true);
          }}
          onSecondary={() => setConfirmation(null)}
          primaryDisabled={!canValidate}
        />
      )}
      {configChangeRequest && (
        <ConfirmationDialog
          title={configChangeRequest.kind === "provider" && configChangeRequest.firstSaved
            ? providerMessages.firstProviderApplyTitle
            : providerMessages.configChangeTitle}
          message={configChangeMessage(configChangeRequest)}
          primaryLabel={configChangeRequest.kind === "provider" && configChangeRequest.firstSaved
            ? providerMessages.applyProvider
            : providerMessages.configChangePrimary}
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
          continueSave={saveAfterValidation.current}
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
          action={wslAction}
          failure={wslFailure}
          feedback={wslFeedback}
          onEnvironmentChange={(id) => {
            setWslEnvironmentId(id);
            setWslProviderId(wslTargetProviderId(
              wslEnvironments.find((item) => item.environmentId === id),
              providers,
            ));
            setWslFailure(null);
            setWslFeedback("");
          }}
          onProviderChange={setWslProviderId}
          onRefresh={() => void refreshWslDialog()}
          onActualRefresh={() => void refreshSelectedWslActualState()}
          onApply={() => void applyWslSelection()}
          onClose={() => {
            if (!wslBusy) setWslDialogOpen(false);
          }}
        />
      )}
      {linuxExportStep === "shell" && (
        <LinuxShellDialog
          shell={linuxExportShell}
          busy={linuxExportBusy}
          failure={linuxExportFailure}
          onShellChange={setLinuxExportShell}
          onChooseLocation={() => void chooseLinuxExportPath()}
          onClose={closeLinuxExport}
        />
      )}
      {linuxExportStep === "success" && linuxExportResult && (
        <LinuxExportSuccessDialog
          shell={linuxExportShell}
          result={linuxExportResult}
          onClose={closeLinuxExport}
        />
      )}
    </main>
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

function RecommendedBadge({ onVisit }: { onVisit: () => Promise<void> }) {
  return (
    <button
      className="recommended-badge recommended-badge-link"
      type="button"
      onClick={() => void onVisit()}
      aria-label={providerMessages.visitDaywayWebsiteAccessibleName}
      title={providerMessages.visitDaywayWebsite}
    >
      推荐
      <ExternalLink size={12} aria-hidden="true" />
    </button>
  );
}

function WslActions({
  environments,
  providers,
  state,
  busy,
  onOpen,
  onExport,
}: {
  environments: WslEnvironmentSummary[];
  providers: ProviderSummary[];
  state: "loading" | "ready" | "error";
  busy: boolean;
  onOpen: () => void;
  onExport: () => void;
}) {
  const manageable = environments.some(isManageableWslEnvironment);
  const currentProvider = environments.find(isManageableWslEnvironment)?.currentProvider ?? null;
  const description = state === "loading"
    ? "正在读取 WSL2 发行版状态。"
    : state === "error"
      ? providerMessages.wslReadFailed
      : manageable
        ? "选择一个 WSL2 发行版和已验证供应商。"
        : providerMessages.wslNoManageable;
  return (
    <>
      <div className="environment-action-row wsl-provider-action-row">
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
        {currentProvider ? (
          <dl className="wsl-provider-summary" aria-label={providerMessages.wslSelectedProvider}>
            <div>
              <dt>{providerMessages.providerName}</dt>
              <dd title={currentProvider.name}>{currentProvider.name}</dd>
            </div>
            <div>
              <dt>{providerMessages.baseUrl}</dt>
              <dd title={currentProvider.baseUrl}>{currentProvider.baseUrl}</dd>
            </div>
            <div>
              <dt>{providerMessages.defaultModel}</dt>
              <dd title={currentProvider.defaultModel}>{currentProvider.defaultModel}</dd>
            </div>
          </dl>
        ) : (
          <span className="wsl-provider-empty">{providerMessages.wslNoProvider}</span>
        )}
      </div>
      <div className="environment-action-row">
        <button
          className="secondary-button environment-command"
          type="button"
          onClick={onExport}
          disabled={busy || providers.length === 0}
        >
          <Download size={17} aria-hidden="true" />
          {providerMessages.exportLinuxScript}
        </button>
      </div>
    </>
  );
}

function wslTargetProviderId(
  environment: WslEnvironmentSummary | undefined,
  providers: ProviderSummary[],
): string | null {
  const currentProviderId = environment?.currentProvider?.id;
  return providers.some((provider) => provider.id === currentProviderId)
    ? currentProviderId ?? null
    : providers[0]?.id ?? null;
}

function LinuxShellDialog({
  shell,
  busy,
  failure,
  onShellChange,
  onChooseLocation,
  onClose,
}: {
  shell: LinuxShell;
  busy: boolean;
  failure: LinuxExportFailure | null;
  onShellChange: (shell: LinuxShell) => void;
  onChooseLocation: () => void;
  onClose: () => void;
}) {
  return (
    <div className="dialog-backdrop">
      <section className="confirmation-dialog linux-export-dialog" role="dialog" aria-modal="true" aria-labelledby="linux-export-title">
        <div className="dialog-heading">
          <div>
            <h2 id="linux-export-title">{providerMessages.exportLinuxScript}</h2>
            <p>{providerMessages.linuxExportShellSubtitle}</p>
          </div>
          <button className="field-icon-button" type="button" onClick={onClose} disabled={busy} aria-label={providerMessages.linuxExportCancel}>
            <X size={18} aria-hidden="true" />
          </button>
        </div>
        <fieldset className="linux-shell-options">
          <legend>{providerMessages.linuxExportShellLegend}</legend>
          {(["bash", "zsh"] as const).map((option) => (
            <label key={option}>
              <input
                type="radio"
                name="linux-shell"
                checked={shell === option}
                onChange={() => onShellChange(option)}
              />
              <span>{linuxShellPresentation[option].displayName}</span>
            </label>
          ))}
        </fieldset>
        <div className="linux-export-sensitive-note">
          <ShieldAlert size={18} aria-hidden="true" />
          <div>
            <strong>{providerMessages.linuxExportSensitiveTitle}</strong>
            <p>{providerMessages.linuxExportSensitiveMessage}</p>
          </div>
        </div>
        {failure && <p className="inline-error" role="alert">{linuxExportFailureMessage(failure)}</p>}
        <div className="dialog-actions">
          <button className="secondary-button" type="button" onClick={onClose} disabled={busy}>{providerMessages.linuxExportCancel}</button>
          <button className="command-button" type="button" onClick={onChooseLocation} disabled={busy}>{providerMessages.linuxExportChooseLocation}</button>
        </div>
      </section>
    </div>
  );
}

function LinuxExportSuccessDialog({
  shell,
  result,
  onClose,
}: {
  shell: LinuxShell;
  result: LinuxExportResult;
  onClose: () => void;
}) {
  const presentation = linuxShellPresentation[shell];
  const fileName = result.suggestedFileName;
  return (
    <div className="dialog-backdrop">
      <section className="confirmation-dialog linux-export-dialog" role="dialog" aria-modal="true" aria-labelledby="linux-export-success-title">
        <h2 id="linux-export-success-title">{providerMessages.linuxExportSuccessTitle(presentation.titleName)}</h2>
        <p>{providerMessages.linuxExportSuccessMessage(result.providerCount)}</p>
        <dl className="linux-export-instructions">
          <div><dt>{providerMessages.linuxExportPermissions}</dt><dd><code>chmod 600 ./{fileName}</code></dd></div>
          <div><dt>{providerMessages.linuxExportDirect}</dt><dd><code>{shell} ./{fileName}</code></dd></div>
          <div><dt>{providerMessages.linuxExportCurrentSession}</dt><dd><code>source ./{fileName}</code></dd></div>
          <div><dt>{providerMessages.linuxExportStartupFile(presentation.startupFile)}</dt><dd><code>source /trusted/path/{fileName}</code></dd></div>
        </dl>
        <dl className="linux-command-instructions" aria-label={providerMessages.linuxExportCommands}>
          <div><dt><code>gpteasy</code></dt><dd>{providerMessages.linuxExportCommandSelect}</dd></div>
          <div><dt><code>gpteasy current</code></dt><dd>{providerMessages.linuxExportCommandCurrent}</dd></div>
          <div><dt><code>gpteasy restore</code></dt><dd>{providerMessages.linuxExportCommandRestore}</dd></div>
          <div><dt><code>gpteasy info</code></dt><dd>{providerMessages.linuxExportCommandInfo}</dd></div>
          <div><dt><code>gpteasy unlock</code></dt><dd>{providerMessages.linuxExportCommandUnlock}</dd></div>
        </dl>
        <div className="dialog-actions">
          <button className="command-button" type="button" onClick={onClose}>{providerMessages.linuxExportDone}</button>
        </div>
      </section>
    </div>
  );
}

function linuxExportFailureMessage(failure: LinuxExportFailure): string {
  const messages: Record<string, string> = {
    "linux_export.no_verified_providers": providerMessages.linuxExportNoProviders,
    "linux_export.overwrite_confirmation_required": providerMessages.linuxExportOverwriteRequired,
    "linux_export.unsafe_destination": providerMessages.linuxExportUnsafeDestination,
    "linux_export.state_unavailable": providerMessages.catalogUnavailable,
    "linux_export.snapshot_invalid": providerMessages.linuxExportSnapshotInvalid,
    "linux_export.concurrent_modification": providerMessages.linuxExportConcurrentModification,
    "linux_export.write_failed": providerMessages.linuxExportWriteFailed,
  };
  return messages[failure.messageId] ?? providerMessages.linuxExportWriteFailed;
}

function WslProviderDialog({
  environments,
  providers,
  state,
  selectedEnvironmentId,
  selectedProviderId,
  busy,
  action,
  failure,
  feedback,
  onEnvironmentChange,
  onProviderChange,
  onRefresh,
  onActualRefresh,
  onApply,
  onClose,
}: {
  environments: WslEnvironmentSummary[];
  providers: ProviderSummary[];
  state: "loading" | "ready" | "error";
  selectedEnvironmentId: string | null;
  selectedProviderId: string | null;
  busy: boolean;
  action: "idle" | "applying" | "refreshing";
  failure: { messageId: string } | null;
  feedback: string;
  onEnvironmentChange: (id: string) => void;
  onProviderChange: (id: string) => void;
  onRefresh: () => void;
  onActualRefresh: () => void;
  onApply: () => void;
  onClose: () => void;
}) {
  const manageableEnvironments = environments.filter(isManageableWslEnvironment);
  const selectedEnvironment = manageableEnvironments.find((item) => item.environmentId === selectedEnvironmentId) ?? null;
  const canApply = state === "ready"
    && selectedEnvironment !== null
    && canApplyWslProvider(selectedEnvironment)
    && Boolean(selectedProviderId)
    && !busy;
  const failureMessage = failure ? wslFailureMessages[failure.messageId] ?? wslFailureMessages["wsl.state_unavailable"] : "";
  return (
    <div className="dialog-backdrop">
      <section className="confirmation-dialog wsl-provider-dialog" role="dialog" aria-modal="true" aria-labelledby="wsl-dialog-title">
        <div className="dialog-heading">
          <div>
            <h2 id="wsl-dialog-title">{providerMessages.wslDialogTitle}</h2>
            <p>{providerMessages.wslDialogSubtitle}</p>
          </div>
          <button className="field-icon-button" type="button" onClick={onClose} disabled={busy} aria-label={providerMessages.wslClose}>
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
                {manageableEnvironments.length > 0 ? (
                  <select value={selectedEnvironmentId ?? ""} onChange={(event) => onEnvironmentChange(event.target.value)} disabled={busy}>
                    {manageableEnvironments.map((environment) => (
                      <option key={environment.environmentId} value={environment.environmentId}>
                      {environment.displayName} · {wslAvailabilityMessages[environment.availability]}
                      </option>
                    ))}
                  </select>
                ) : (
                  <span className="wsl-empty-selection">{providerMessages.wslNoManageable}</span>
                )}
              </label>
              <label className="form-field">
                <span>{providerMessages.wslProvider}</span>
                <select value={selectedProviderId ?? ""} onChange={(event) => onProviderChange(event.target.value)} disabled={busy || providers.length === 0}>
                  {providers.length === 0 && <option value="">{providerMessages.wslNoProvider}</option>}
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
                  <span className="pending-badge">
                    {wslConfigurationStateMessages[selectedEnvironment.configurationState ?? "unknown"]}
                  </span>
                </div>
                {selectedEnvironment.currentProvider && <p>当前供应商：{selectedEnvironment.currentProvider.name}</p>}
                {selectedEnvironment.configurationState === "provider_missing" && selectedEnvironment.actualProviderId && (
                  <p>当前供应商 ID：<code>{selectedEnvironment.actualProviderId}</code></p>
                )}
                {selectedEnvironment.defaultUid !== null && <p>默认用户 UID：{selectedEnvironment.defaultUid}</p>}
                {selectedEnvironment.running
                  ? <p>{providerMessages.wslRunningWarning}</p>
                  : <p>{providerMessages.wslStoppedWarning}</p>}
                <p>{providerMessages.wslDefaultUserScope}</p>
              </div>
            )}
            {failure && <p className="inline-error" role="alert">{failureMessage}</p>}
            {feedback && <p className="catalog-feedback" role="status">{feedback}</p>}
          </>
        )}
        <div className="dialog-actions">
          <button className="secondary-button wsl-back-button" type="button" onClick={onClose} disabled={busy}>{providerMessages.wslBack}</button>
          <button
            className="secondary-button wsl-refresh-button"
            type="button"
            onClick={onActualRefresh}
            disabled={state !== "ready" || selectedEnvironment === null || busy}
          >
            {action === "refreshing"
              ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" />
              : <RefreshCw size={17} aria-hidden="true" />}
            {action === "refreshing"
              ? providerMessages.wslRefreshingActual
              : providerMessages.wslRefreshActual}
          </button>
          <button className="command-button wsl-apply-button" type="button" onClick={onApply} disabled={!canApply}>
            {action === "applying" ? <LoaderCircle className="is-spinning" size={17} aria-hidden="true" /> : <Check size={17} aria-hidden="true" />}
            {action === "applying" ? providerMessages.wslApplying : providerMessages.wslApply}
          </button>
        </div>
      </section>
    </div>
  );
}

function lifecycleOutcomeMessage(outcome: WslLifecycleOutcome | undefined): string {
  switch (outcome) {
    case "unchanged_running":
      return providerMessages.wslLifecycleUnchangedRunning;
    case "stopped_naturally":
      return providerMessages.wslLifecycleStoppedNaturally;
    case "still_running":
      return providerMessages.wslLifecycleStillRunning;
    default:
      return "";
  }
}

function lifecycleResultsMessage(results: WslLifecycleResult[]): string {
  const stopped = results.filter((result) => result.outcome === "stopped_naturally").length;
  const stillRunning = results.filter((result) => result.outcome === "still_running").length;
  if (stillRunning > 0) return providerMessages.wslLifecycleStillRunning;
  if (stopped > 0) return providerMessages.wslLifecycleStoppedNaturally;
  return results.length > 0 ? providerMessages.wslLifecycleUnchangedRunning : "";
}

function openAiLoginReason(snapshot: EnvironmentSnapshot | null): string {
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
  return openAiReason;
}

function configChangeMessage(request: ConfigChangeRequest): string {
  const risk = providerMessages.configChangeConsumerRisk;
  if (request.kind === "provider") {
    if (request.firstSaved) {
      return `${providerMessages.firstProviderApplyMessage(request.provider.name)}${risk}`;
    }
    return `${providerMessages.configChangeProviderTarget(request.provider.name)}${risk}`;
  }
  if (request.kind === "provider_update") {
    return `${providerMessages.configChangeProviderUpdateTarget(request.provider.name)}${risk}`;
  }
  return `${providerMessages.configChangeOpenAiTarget}${risk}`;
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
  continueSave,
  onAccept,
  onReject,
}: {
  requestedBaseUrl: string;
  suggestedBaseUrl: string;
  continueSave: boolean;
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
        <p>
          {continueSave
            ? providerMessages.addressSuggestionSaveMessage
            : providerMessages.addressSuggestionMessage}
        </p>
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
