import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Archive as ArchiveIcon,
  ArchiveRestore,
  ArrowLeft,
  ChevronDown,
  Download,
  LoaderCircle,
  RefreshCw,
  Search,
  ShieldAlert,
  Trash2,
} from "lucide-react";

import AppSidebar, { type OpenAiSidebarAction, type UpdateSidebarState } from "./AppSidebar";
import {
  archiveSessions,
  asSessionFailure,
  cancelSessionRequest,
  chooseSessionExportDestination,
  deleteSession,
  enterSessionManagement,
  exportSessionMarkdown,
  leaveSessionManagement,
  listSessions,
  readSession,
  unarchiveSessions,
  type SessionAvailability,
  type SessionAvailabilityStatus,
  type SessionDetail,
  type SessionFailure,
  type SessionMutationAvailability,
  type SessionMutationResult,
  type SessionQuery,
  type SessionSummary,
} from "./contracts/session";
import { sessionMessages } from "./messages";

type SessionTab = "active" | "archived";
type ListState = "initial_loading" | "ready" | "loading_more" | "error";
type DetailState =
  | { kind: "list" }
  | { kind: "loading"; summary: SessionSummary }
  | { kind: "loaded"; detail: SessionDetail }
  | { kind: "error"; summary: SessionSummary; failure: SessionFailure };

interface SessionListCache {
  sessions: SessionSummary[];
  nextCursor: string | null;
}

interface MutationOutcome {
  sessionId: string;
  title: string;
  actualState: SessionMutationResult["actualState"];
}

export default function SessionPage({
  active = true,
  onOpenProviders,
  openAiAction,
  currentProviderName,
  update,
}: {
  active?: boolean;
  onOpenProviders: () => void;
  openAiAction?: OpenAiSidebarAction;
  currentProviderName?: string | null;
  update?: UpdateSidebarState;
}) {
  const leaseId = useRef(`session-page-${createLeaseId()}`);
  const requestGeneration = useRef(0);
  const activeListRequests = useRef(new Set<string>());
  const listCache = useRef(new Map<string, SessionListCache>());
  const leaseActive = useRef(false);
  const activeRef = useRef(active);
  const detailRequestGeneration = useRef(0);
  const listScrollPosition = useRef(0);
  const tabRef = useRef<SessionTab>("active");
  const [availability, setAvailability] = useState<SessionAvailability | null>(null);
  const [tab, setTab] = useState<SessionTab>("active");
  const [searchTerm, setSearchTerm] = useState("");
  const [project, setProject] = useState("");
  const [modelProvider, setModelProvider] = useState("");
  const [sessions, setSessions] = useState<SessionSummary[]>([]);
  const [knownProjects, setKnownProjects] = useState<string[]>([]);
  const [knownProviders, setKnownProviders] = useState<string[]>([]);
  const [nextCursor, setNextCursor] = useState<string | null>(null);
  const [listState, setListState] = useState<ListState>("initial_loading");
  const [listFailure, setListFailure] = useState<SessionFailure | null>(null);
  const [listRetry, setListRetry] = useState(0);
  const [detailState, setDetailState] = useState<DetailState>({ kind: "list" });
  const [exportState, setExportState] = useState<"idle" | "exporting" | "saved" | "error">("idle");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [mutationResults, setMutationResults] = useState<Record<string, SessionMutationResult>>({});
  const [mutationSummary, setMutationSummary] = useState<string | null>(null);
  const [mutationOutcomes, setMutationOutcomes] = useState<MutationOutcome[]>([]);
  const [mutationBusy, setMutationBusy] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<SessionDetail | null>(null);
  const [bulkDeleteTargets, setBulkDeleteTargets] = useState<SessionSummary[]>([]);
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteFailure, setDeleteFailure] = useState<string | null>(null);
  const [deleteLoadingId, setDeleteLoadingId] = useState<string | null>(null);

  useEffect(() => {
    activeRef.current = active;
  }, [active]);

  const cancelInFlight = useCallback(() => {
    for (const requestId of activeListRequests.current) {
      activeListRequests.current.delete(requestId);
      void cancelSessionRequest(requestId);
    }
  }, []);

  const checkAvailability = useCallback(async () => {
    const generation = ++requestGeneration.current;
    detailRequestGeneration.current += 1;
    setAvailability(null);
    try {
      const nextAvailability = await enterSessionManagement(leaseId.current);
      if (requestGeneration.current !== generation || !activeRef.current) {
        if (!activeRef.current) void leaveSessionManagement(leaseId.current);
        return;
      }
      leaseActive.current = true;
      setAvailability(nextAvailability);
    } catch (error) {
      if (requestGeneration.current !== generation || !activeRef.current) return;
      const failure = asSessionFailure(error);
      setAvailability({
        status: failure.category === "recovery_failed" ? "recovery_failed" : "initialization_failed",
        messageId: failure.messageId,
        codexVersion: null,
        mutation: {
          status: "unavailable",
          messageId: "session.mutations_unavailable",
        },
      });
    }
  }, []);

  useEffect(() => {
    const activeLease = leaseId.current;
    if (!active) {
      requestGeneration.current += 1;
      detailRequestGeneration.current += 1;
      cancelInFlight();
      if (leaseActive.current) {
        leaseActive.current = false;
        void leaveSessionManagement(activeLease);
      }
      setAvailability(null);
      return;
    }

    listCache.current.clear();
    void checkAvailability();
    return () => {
      requestGeneration.current += 1;
      detailRequestGeneration.current += 1;
      cancelInFlight();
      if (leaseActive.current) {
        leaseActive.current = false;
        void leaveSessionManagement(activeLease);
      }
    };
  }, [active, cancelInFlight, checkAvailability]);

  const baseQuery = useMemo<Omit<SessionQuery, "cursor" | "requestId">>(() => ({
    archived: tab === "archived",
    searchTerm: searchTerm.trim() || null,
    project: project || null,
    modelProvider: modelProvider || null,
    limit: 40,
  }), [modelProvider, project, searchTerm, tab]);
  const listQueryKey = JSON.stringify(baseQuery);

  function refreshList() {
    listCache.current.delete(listQueryKey);
    setListRetry((current) => current + 1);
  }

  useEffect(() => {
    tabRef.current = tab;
  }, [tab]);

  useEffect(() => {
    if (!active || availability?.status !== "available") return;
    cancelInFlight();
    const cached = listCache.current.get(listQueryKey);
    if (cached) {
      setSessions(cached.sessions);
      setNextCursor(cached.nextCursor);
      setListFailure(null);
      setListState("ready");
      setSelectedIds(new Set());
      setMutationResults({});
      setMutationSummary(null);
      setMutationOutcomes([]);
      return cancelInFlight;
    }
    listCache.current.delete(listQueryKey);
    const generation = ++requestGeneration.current;
    setListState("initial_loading");
    setListFailure(null);
    setSessions([]);
    setSelectedIds(new Set());
    setMutationResults({});
    setMutationSummary(null);
    setMutationOutcomes([]);
    setNextCursor(null);
    const requestId = `session-list-${createLeaseId()}`;
    activeListRequests.current.add(requestId);
    void listSessions({ ...baseQuery, requestId, cursor: null })
      .then((page) => {
        if (requestGeneration.current !== generation) return;
        const nextSessions = uniqueSessions(page.sessions);
        listCache.current.set(listQueryKey, { sessions: nextSessions, nextCursor: page.nextCursor });
        setSessions(nextSessions);
        rememberFacets(page.sessions, setKnownProjects, setKnownProviders);
        setNextCursor(page.nextCursor);
        setListState("ready");
      })
      .catch((error) => {
        if (requestGeneration.current !== generation) return;
        setListFailure(asSessionFailure(error));
        setListState("error");
      })
      .finally(() => activeListRequests.current.delete(requestId));
    return cancelInFlight;
  }, [active, availability?.status, baseQuery, cancelInFlight, listQueryKey, listRetry]);

  async function loadMore() {
    if (!nextCursor || listState === "loading_more") return;
    const generation = requestGeneration.current;
    const requestId = `session-list-${createLeaseId()}`;
    activeListRequests.current.add(requestId);
    setListState("loading_more");
    setListFailure(null);
    try {
      const page = await listSessions({ ...baseQuery, requestId, cursor: nextCursor });
      if (requestGeneration.current !== generation) return;
      setSessions((current) => {
        const nextSessions = uniqueSessions([...current, ...page.sessions]);
        listCache.current.set(listQueryKey, { sessions: nextSessions, nextCursor: page.nextCursor });
        return nextSessions;
      });
      rememberFacets(page.sessions, setKnownProjects, setKnownProviders);
      setNextCursor(page.nextCursor);
      setListState("ready");
    } catch (error) {
      if (requestGeneration.current !== generation) return;
      setListFailure(asSessionFailure(error));
      setListState("error");
    } finally {
      activeListRequests.current.delete(requestId);
    }
  }

  async function openDetail(summary: SessionSummary, rememberListScroll = true) {
    if (rememberListScroll) {
      listScrollPosition.current = window.scrollY;
    }
    const generation = ++detailRequestGeneration.current;
    setDetailState({ kind: "loading", summary });
    setExportState("idle");
    try {
      const detail = await readSession(summary.id);
      if (detailRequestGeneration.current !== generation) return;
      setDetailState({ kind: "loaded", detail });
    } catch (error) {
      if (detailRequestGeneration.current !== generation) return;
      setDetailState({ kind: "error", summary, failure: asSessionFailure(error) });
    }
  }

  function returnToList() {
    detailRequestGeneration.current += 1;
    setDetailState({ kind: "list" });
    if (listScrollPosition.current > 0) {
      requestAnimationFrame(() => window.scrollTo({ top: listScrollPosition.current }));
    }
  }

  async function exportMarkdown(detail: SessionDetail) {
    setExportState("exporting");
    try {
      const destination = await chooseSessionExportDestination(detail.title);
      if (!destination) {
        setExportState("idle");
        return;
      }
      await exportSessionMarkdown(detail, destination);
      setExportState("saved");
    } catch {
      setExportState("error");
    }
  }

  async function mutateSessions(sessionIds: string[], action: "archive" | "unarchive") {
    if (sessionIds.length === 0 || mutationBusy) return;
    setMutationBusy(true);
    setMutationSummary(null);
    try {
      const results = action === "archive"
        ? await archiveSessions(sessionIds)
        : await unarchiveSessions(sessionIds);
      applyMutationResults(results, action);
    } catch {
      setMutationSummary(sessionMessages.mutationFailed);
    } finally {
      setMutationBusy(false);
    }
  }

  function applyMutationResults(
    results: SessionMutationResult[],
    action: "archive" | "unarchive",
  ) {
    const byId = new Map(results.map((result) => [result.sessionId, result]));
    const titles = new Map(sessions.map((session) => [session.id, session.title]));
    if (detailState.kind === "loaded") {
      titles.set(detailState.detail.id, detailState.detail.title);
    }
    setMutationOutcomes(results.map((result) => ({
      sessionId: result.sessionId,
      title: titles.get(result.sessionId) ?? result.sessionId,
      actualState: result.actualState,
    })));
    setSessions((current) => {
      const nextSessions = current.filter((session) => {
        const result = byId.get(session.id);
        return !result || stateBelongsToTab(result.actualState, tabRef.current);
      });
      listCache.current.set(listQueryKey, { sessions: nextSessions, nextCursor });
      return nextSessions;
    });
    setSelectedIds(new Set(results
      .filter((result) => result.status !== "succeeded" && stateBelongsToTab(result.actualState, tabRef.current))
      .map((result) => result.sessionId)));
    setMutationResults(Object.fromEntries(results.map((result) => [result.sessionId, result])));
    const succeeded = results.filter((result) => result.status === "succeeded").length;
    const failed = results.length - succeeded;
    setMutationSummary(action === "archive"
      ? failed > 0
        ? sessionMessages.partialArchive(succeeded, failed)
        : sessionMessages.archiveComplete(succeeded)
      : failed > 0
        ? sessionMessages.partialUnarchive(succeeded, failed)
        : sessionMessages.unarchiveComplete(succeeded));
    const detailResult = detailState.kind === "loaded"
      ? byId.get(detailState.detail.id)
      : undefined;
    if (detailResult && !stateBelongsToTab(detailResult.actualState, tabRef.current)) {
      returnToList();
    }
  }

  async function openDeleteConfirmation(summary: SessionSummary) {
    if (deleteLoadingId) return;
    setDeleteLoadingId(summary.id);
    setDeleteFailure(null);
    try {
      const detail = detailState.kind === "loaded" && detailState.detail.id === summary.id
        ? detailState.detail
        : await readSession(summary.id);
      setDeleteTarget(detail);
    } catch {
      setMutationSummary(sessionMessages.detailFailed);
    } finally {
      setDeleteLoadingId(null);
    }
  }

  async function confirmDelete() {
    if (!deleteTarget || deleteBusy) return;
    setDeleteBusy(true);
    setDeleteFailure(null);
    try {
      const result = await deleteSession(deleteTarget.id);
      setSessions((current) => {
        const nextSessions = current.filter((session) => (
          session.id !== result.sessionId || stateBelongsToTab(result.actualState, tabRef.current)
        ));
        listCache.current.set(listQueryKey, { sessions: nextSessions, nextCursor });
        return nextSessions;
      });
      if (result.actualState === "deleted") {
        setDeleteTarget(null);
        setDetailState({ kind: "list" });
        setSelectedIds((current) => {
          const next = new Set(current);
          next.delete(result.sessionId);
          return next;
        });
        setMutationSummary(sessionMessages.deleteOutcome(result.actualState));
      } else {
        setDeleteFailure(sessionMessages.deleteOutcome(result.actualState));
      }
    } catch {
      setDeleteFailure(sessionMessages.deleteFailed);
    } finally {
      setDeleteBusy(false);
    }
  }

  async function confirmBulkDelete() {
    if (bulkDeleteTargets.length === 0 || deleteBusy) return;
    setDeleteBusy(true);
    setDeleteFailure(null);
    const results: SessionMutationResult[] = [];
    for (const target of bulkDeleteTargets) {
      try {
        results.push(await deleteSession(target.id));
      } catch {
        results.push({
          sessionId: target.id,
          status: "failed",
          actualState: "unknown",
          messageId: "session.request_failed",
        });
      }
    }

    const byId = new Map(results.map((result) => [result.sessionId, result]));
    const titles = new Map(bulkDeleteTargets.map((target) => [target.id, target.title]));
    setSessions((current) => {
      const nextSessions = current.filter((session) => {
        const result = byId.get(session.id);
        return !result || stateBelongsToTab(result.actualState, tabRef.current);
      });
      listCache.current.set(listQueryKey, { sessions: nextSessions, nextCursor });
      return nextSessions;
    });
    setSelectedIds(new Set(results
      .filter((result) => stateBelongsToTab(result.actualState, tabRef.current))
      .map((result) => result.sessionId)));
    setMutationOutcomes(results.map((result) => ({
      sessionId: result.sessionId,
      title: titles.get(result.sessionId) ?? result.sessionId,
      actualState: result.actualState,
    })));
    const deleted = results.filter((result) => result.actualState === "deleted").length;
    const failed = results.length - deleted;
    setMutationSummary(failed > 0
      ? sessionMessages.partialDelete(deleted, failed)
      : sessionMessages.deleteComplete(deleted));
    setBulkDeleteTargets([]);
    setDeleteBusy(false);
  }

  if (!availability) {
    return (
      <SessionShell onOpenProviders={onOpenProviders} openAiAction={openAiAction} currentProviderName={currentProviderName} update={update}>
        <div className="loading-state" role="status">
          <LoaderCircle className="is-spinning" size={22} aria-hidden="true" />
          <span>{sessionMessages.loading}</span>
        </div>
      </SessionShell>
    );
  }

  if (availability.status !== "available") {
    return (
      <SessionShell onOpenProviders={onOpenProviders} openAiAction={openAiAction} currentProviderName={currentProviderName} update={update}>
        <UnavailableState availability={availability} onRetry={() => void checkAvailability()} />
      </SessionShell>
    );
  }

  if (
    detailState.kind === "list" &&
    sessions.length === 0 &&
    (listFailure?.category === "incompatible" || listFailure?.category === "recovery_failed")
  ) {
    const status = listFailure.category === "incompatible" ? "incompatible" : "recovery_failed";
    return (
      <SessionShell onOpenProviders={onOpenProviders} openAiAction={openAiAction} currentProviderName={currentProviderName} update={update}>
        <UnavailableState
          availability={{
            status,
            messageId: listFailure.messageId,
            codexVersion: availability.codexVersion,
            mutation: availability.mutation,
          }}
          onRetry={() => void checkAvailability()}
        />
      </SessionShell>
    );
  }

  return (
    <SessionShell onOpenProviders={onOpenProviders} openAiAction={openAiAction} currentProviderName={currentProviderName} update={update}>
      {detailState.kind === "list" ? (
        <SessionList
          tab={tab}
          setTab={setTab}
          searchTerm={searchTerm}
          setSearchTerm={setSearchTerm}
          project={project}
          setProject={setProject}
          modelProvider={modelProvider}
          setModelProvider={setModelProvider}
          projects={knownProjects}
          providers={knownProviders}
          sessions={sessions}
          listState={listState}
          listFailure={listFailure}
          hasQuery={Boolean(searchTerm.trim() || project || modelProvider)}
          nextCursor={nextCursor}
          onLoadMore={() => void loadMore()}
          onRefresh={refreshList}
          onRetry={refreshList}
          onRecover={() => void checkAvailability()}
          onOpen={(summary) => void openDetail(summary)}
          mutation={availability.mutation}
          selectedIds={selectedIds}
          mutationResults={mutationResults}
          mutationSummary={mutationSummary}
          mutationOutcomes={mutationOutcomes}
          mutationBusy={mutationBusy}
          deleteLoadingId={deleteLoadingId}
          onToggleSelected={(sessionId) => setSelectedIds((current) => toggleSelection(current, sessionId))}
          onToggleAll={() => setSelectedIds((current) => (
            current.size === sessions.length ? new Set() : new Set(sessions.map((session) => session.id))
          ))}
          onMutate={(sessionIds) => void mutateSessions(
            sessionIds,
            tab === "active" ? "archive" : "unarchive",
          )}
          onDelete={(summary) => void openDeleteConfirmation(summary)}
          onDeleteSelected={() => {
            setMutationResults({});
            setMutationSummary(null);
            setMutationOutcomes([]);
            setBulkDeleteTargets(sessions.filter((session) => selectedIds.has(session.id)));
          }}
        />
      ) : (
        <SessionDetailView
          state={detailState}
          exportState={exportState}
          tab={tab}
          mutation={availability.mutation}
          mutationBusy={mutationBusy}
          mutationSummary={mutationSummary}
          mutationOutcomes={mutationOutcomes}
          onBack={returnToList}
          onRetry={(summary) => void openDetail(summary, false)}
          onExport={(detail) => void exportMarkdown(detail)}
          onMutate={(detail) => void mutateSessions(
            [detail.id],
            tab === "active" ? "archive" : "unarchive",
          )}
          onDelete={(detail) => void openDeleteConfirmation(detail)}
        />
      )}
      {deleteTarget && (
        <DeleteSessionDialog
          detail={deleteTarget}
          targets={[deleteTarget]}
          knownDescendants={knownDescendants(deleteTarget.id, sessions)}
          busy={deleteBusy}
          failure={deleteFailure}
          exportState={exportState}
          onCancel={() => {
            setDeleteTarget(null);
            setDeleteFailure(null);
          }}
          onExport={() => void exportMarkdown(deleteTarget)}
          onConfirm={() => void confirmDelete()}
        />
      )}
      {bulkDeleteTargets.length > 0 && (
        <DeleteSessionDialog
          detail={null}
          targets={bulkDeleteTargets}
          knownDescendants={[]}
          busy={deleteBusy}
          failure={deleteFailure}
          exportState="idle"
          onCancel={() => {
            setBulkDeleteTargets([]);
            setDeleteFailure(null);
          }}
          onConfirm={() => void confirmBulkDelete()}
        />
      )}
    </SessionShell>
  );
}

function SessionShell({
  children,
  onOpenProviders,
  openAiAction,
  currentProviderName,
  update,
}: {
  children: React.ReactNode;
  onOpenProviders: () => void;
  openAiAction?: OpenAiSidebarAction;
  currentProviderName?: string | null;
  update?: UpdateSidebarState;
}) {
  return (
    <div className="app-shell">
      <AppSidebar
        activeView="sessions"
        onOpenProviders={onOpenProviders}
        onOpenSessions={() => undefined}
        openAiAction={openAiAction}
        currentProviderName={currentProviderName}
        update={update}
      />
      <main className="main-content session-main">{children}</main>
    </div>
  );
}

function SessionList({
  tab,
  setTab,
  searchTerm,
  setSearchTerm,
  project,
  setProject,
  modelProvider,
  setModelProvider,
  projects,
  providers,
  sessions,
  listState,
  listFailure,
  hasQuery,
  nextCursor,
  onLoadMore,
  onRefresh,
  onRetry,
  onRecover,
  onOpen,
  mutation,
  selectedIds,
  mutationResults,
  mutationSummary,
  mutationOutcomes,
  mutationBusy,
  deleteLoadingId,
  onToggleSelected,
  onToggleAll,
  onMutate,
  onDelete,
  onDeleteSelected,
}: {
  tab: SessionTab;
  setTab: (tab: SessionTab) => void;
  searchTerm: string;
  setSearchTerm: (value: string) => void;
  project: string;
  setProject: (value: string) => void;
  modelProvider: string;
  setModelProvider: (value: string) => void;
  projects: string[];
  providers: string[];
  sessions: SessionSummary[];
  listState: ListState;
  listFailure: SessionFailure | null;
  hasQuery: boolean;
  nextCursor: string | null;
  onLoadMore: () => void;
  onRefresh: () => void;
  onRetry: () => void;
  onRecover: () => void;
  onOpen: (summary: SessionSummary) => void;
  mutation: SessionMutationAvailability;
  selectedIds: Set<string>;
  mutationResults: Record<string, SessionMutationResult>;
  mutationSummary: string | null;
  mutationOutcomes: MutationOutcome[];
  mutationBusy: boolean;
  deleteLoadingId: string | null;
  onToggleSelected: (sessionId: string) => void;
  onToggleAll: () => void;
  onMutate: (sessionIds: string[]) => void;
  onDelete: (summary: SessionSummary) => void;
  onDeleteSelected: () => void;
}) {
  const mutationsAllowed = mutation.status === "allowed";
  const actionLabel = tab === "active" ? sessionMessages.archive : sessionMessages.unarchive;
  const mutationDisabledReason = mutationsAllowed
    ? undefined
    : sessionMessages.mutationBlocked.unavailable;
  return (
    <>
      <header className="page-header">
        <h1>{sessionMessages.pageTitle}</h1>
        <button
          className="icon-button"
          type="button"
          aria-label={sessionMessages.refreshList}
          title={sessionMessages.refreshList}
          onClick={onRefresh}
          disabled={listState === "initial_loading" || listState === "loading_more"}
        >
          <RefreshCw
            className={listState === "initial_loading" ? "is-spinning" : undefined}
            size={17}
            aria-hidden="true"
          />
        </button>
      </header>
      <div className="session-tabs" role="tablist" aria-label="会话范围">
        <button type="button" role="tab" aria-selected={tab === "active"} onClick={() => setTab("active")}>
          {sessionMessages.activeTab}
        </button>
        <button type="button" role="tab" aria-selected={tab === "archived"} onClick={() => setTab("archived")}>
          {sessionMessages.archivedTab}
        </button>
      </div>
      <div className="session-filters" aria-label="会话筛选">
        <label className="session-search">
          <span className="visually-hidden">{sessionMessages.searchLabel}</span>
          <Search size={16} aria-hidden="true" />
          <input
            type="search"
            aria-label={sessionMessages.searchLabel}
            placeholder={sessionMessages.searchPlaceholder}
            value={searchTerm}
            onChange={(event) => setSearchTerm(event.currentTarget.value)}
          />
        </label>
        <label>
          <span>{sessionMessages.projectFilter}</span>
          <input
            type="text"
            list="session-project-options"
            placeholder={sessionMessages.allProjects}
            value={project}
            onChange={(event) => setProject(event.currentTarget.value)}
          />
          <datalist id="session-project-options">
            {projects.map((value) => <option key={value} value={value} />)}
          </datalist>
        </label>
        <label>
          <span>{sessionMessages.providerFilter}</span>
          <input
            type="text"
            list="session-provider-options"
            placeholder={sessionMessages.allProviders}
            value={modelProvider}
            onChange={(event) => setModelProvider(event.currentTarget.value)}
          />
          <datalist id="session-provider-options">
            {providers.map((value) => <option key={value} value={value} />)}
          </datalist>
        </label>
      </div>

      <MutationGate mutation={mutation} />
      {selectedIds.size > 0 && (
        <div className="session-selection-toolbar" aria-label="已选会话操作">
          <span>已选择 {selectedIds.size} 个会话</span>
          <div className="session-selection-actions">
            <button
              className="command-button compact"
              type="button"
              aria-label={tab === "active" ? sessionMessages.archiveSelected : sessionMessages.unarchiveSelected}
              onClick={() => onMutate([...selectedIds])}
              disabled={!mutationsAllowed || mutationBusy}
            >
              {tab === "active"
                ? <ArchiveIcon size={16} aria-hidden="true" />
                : <ArchiveRestore size={16} aria-hidden="true" />}
              {tab === "active" ? sessionMessages.archiveSelected : sessionMessages.unarchiveSelected}
            </button>
            <button
              className="danger-button compact"
              type="button"
              onClick={onDeleteSelected}
              disabled={!mutationsAllowed || mutationBusy}
            >
              <Trash2 size={16} aria-hidden="true" />
              {sessionMessages.deleteSelected}
            </button>
          </div>
        </div>
      )}
      {mutationSummary && <p className="session-mutation-summary" role="status">{mutationSummary}</p>}
      <MutationOutcomeList outcomes={mutationOutcomes} />

      {listState === "initial_loading" && (
        <p className="session-list-state" role="status">
          <LoaderCircle className="is-spinning" size={18} aria-hidden="true" />
          {sessionMessages.loading}
        </p>
      )}
      {listState !== "initial_loading" && sessions.length === 0 && listState !== "error" && (
        <p className="session-list-state">{hasQuery ? sessionMessages.noResults : sessionMessages.empty}</p>
      )}
      {sessions.length > 0 && (
        <div className="session-table-wrap">
          <table className="session-table">
            <thead>
              <tr>
                <th className="session-select-column">
                  <input
                    type="checkbox"
                    aria-label="选择当前已加载会话"
                    checked={sessions.length > 0 && selectedIds.size === sessions.length}
                    onChange={onToggleAll}
                    disabled={!mutationsAllowed || mutationBusy}
                  />
                </th>
                <th>{sessionMessages.titleColumn}</th>
                <th>{sessionMessages.projectColumn}</th>
                <th>{sessionMessages.providerColumn}</th>
                <th>{sessionMessages.sourceColumn}</th>
                <th>{sessionMessages.updatedColumn}</th>
                <th className="session-action-column">操作</th>
              </tr>
            </thead>
            <tbody>
              {sessions.map((session) => {
                const result = mutationResults[session.id];
                const retryLabel = tab === "active"
                  ? sessionMessages.retryArchive
                  : sessionMessages.retryUnarchive;
                return (
                <tr key={session.id}>
                  <td className="session-select-column">
                    <input
                      type="checkbox"
                      aria-label={`选择会话：${session.title}`}
                      checked={selectedIds.has(session.id)}
                      onChange={() => onToggleSelected(session.id)}
                      disabled={!mutationsAllowed || mutationBusy}
                    />
                  </td>
                  <td>
                    <button
                      className="session-title-button"
                      type="button"
                      aria-label={`打开会话：${session.title}`}
                      onClick={() => onOpen(session)}
                    >
                      <strong>{session.title}</strong>
                      <span>{session.preview}</span>
                    </button>
                  </td>
                  <td title={session.project}><span className="truncate-cell">{session.project}</span></td>
                  <td title={session.modelProvider}><span className="truncate-cell">{session.modelProvider || "未知"}</span></td>
                  <td>{session.source}</td>
                  <td><time dateTime={epochDateTime(session.updatedAt)}>{formatTimestamp(session.updatedAt)}</time></td>
                  <td className="session-row-actions">
                    <div className="session-row-action-buttons">
                    {result?.status === "failed" ? (
                      <button
                        className="secondary-button compact"
                        type="button"
                        aria-label={`${retryLabel}：${session.title}`}
                        title={mutationDisabledReason ?? `${retryLabel}：${session.title}`}
                        onClick={() => onMutate([session.id])}
                        disabled={!mutationsAllowed || mutationBusy}
                      >
                        <RefreshCw size={15} aria-hidden="true" />
                      </button>
                    ) : (
                      <button
                        className="icon-button"
                        type="button"
                        aria-label={`${actionLabel}会话：${session.title}`}
                        title={mutationDisabledReason ?? `${actionLabel}会话：${session.title}`}
                        onClick={() => onMutate([session.id])}
                        disabled={!mutationsAllowed || mutationBusy}
                      >
                        {tab === "active"
                          ? <ArchiveIcon size={16} aria-hidden="true" />
                          : <ArchiveRestore size={16} aria-hidden="true" />}
                      </button>
                    )}
                    <button
                      className="icon-button danger-icon-button"
                      type="button"
                      aria-label={`${sessionMessages.delete}：${session.title}`}
                      title={mutationDisabledReason ?? `${sessionMessages.delete}：${session.title}`}
                      onClick={() => onDelete(session)}
                      disabled={!mutationsAllowed || mutationBusy || deleteLoadingId !== null}
                    >
                      {deleteLoadingId === session.id
                        ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                        : <Trash2 size={16} aria-hidden="true" />}
                    </button>
                    </div>
                  </td>
                </tr>
              );})}
            </tbody>
          </table>
        </div>
      )}
      {listState === "error" && (
        <div className="session-inline-error" role="alert">
          <span>{failureMessage(listFailure)}</span>
          <button
            className="secondary-button compact"
            type="button"
            onClick={listFailure?.category === "recovery_failed"
              ? onRecover
              : nextCursor
                ? onLoadMore
                : onRetry}
          >
            <RefreshCw size={15} aria-hidden="true" />
            {sessionMessages.retry}
          </button>
        </div>
      )}
      {nextCursor && listState !== "error" && (
        <button className="secondary-button session-load-more" type="button" onClick={onLoadMore} disabled={listState === "loading_more"}>
          {listState === "loading_more"
            ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
            : <ChevronDown size={16} aria-hidden="true" />}
          {listState === "loading_more" ? sessionMessages.loadingMore : sessionMessages.loadMore}
        </button>
      )}
    </>
  );
}

function SessionDetailView({
  state,
  exportState,
  tab,
  mutation,
  mutationBusy,
  mutationSummary,
  mutationOutcomes,
  onBack,
  onRetry,
  onExport,
  onMutate,
  onDelete,
}: {
  state: Exclude<DetailState, { kind: "list" }>;
  exportState: "idle" | "exporting" | "saved" | "error";
  tab: SessionTab;
  mutation: SessionMutationAvailability;
  mutationBusy: boolean;
  mutationSummary: string | null;
  mutationOutcomes: MutationOutcome[];
  onBack: () => void;
  onRetry: (summary: SessionSummary) => void;
  onExport: (detail: SessionDetail) => void;
  onMutate: (detail: SessionDetail) => void;
  onDelete: (detail: SessionDetail) => void;
}) {
  const summary = state.kind === "loaded" ? state.detail : state.summary;
  const mutationsAllowed = mutation.status === "allowed";
  const mutationDisabledReason = mutationsAllowed
    ? undefined
    : sessionMessages.mutationBlocked.unavailable;
  return (
    <>
      <header className="page-header session-detail-header">
        <button className="icon-button" type="button" aria-label={sessionMessages.backToList} title={sessionMessages.backToList} onClick={onBack}>
          <ArrowLeft size={18} aria-hidden="true" />
        </button>
        <h1>{summary.title}</h1>
        {state.kind === "loaded" && (
          <div className="session-detail-actions">
            <button className="command-button compact" type="button" onClick={() => onExport(state.detail)} disabled={exportState === "exporting"}>
              {exportState === "exporting"
                ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                : <Download size={16} aria-hidden="true" />}
              {exportState === "exporting" ? sessionMessages.exporting : sessionMessages.exportMarkdown}
            </button>
            <button
              className="secondary-button compact"
              type="button"
              title={mutationDisabledReason ?? `${tab === "active" ? sessionMessages.archive : sessionMessages.unarchive}会话`}
              onClick={() => onMutate(state.detail)}
              disabled={!mutationsAllowed || mutationBusy}
            >
              {tab === "active"
                ? <ArchiveIcon size={16} aria-hidden="true" />
                : <ArchiveRestore size={16} aria-hidden="true" />}
              {tab === "active" ? sessionMessages.archive : sessionMessages.unarchive}会话
            </button>
            <button
              className="danger-button compact"
              type="button"
              title={mutationDisabledReason ?? sessionMessages.delete}
              onClick={() => onDelete(state.detail)}
              disabled={!mutationsAllowed || mutationBusy}
            >
              <Trash2 size={16} aria-hidden="true" />
              {sessionMessages.delete}
            </button>
          </div>
        )}
      </header>
      <dl className="session-metadata">
        <div><dt>{sessionMessages.projectColumn}</dt><dd title={summary.project}>{summary.project}</dd></div>
        <div><dt>{sessionMessages.providerColumn}</dt><dd>{summary.modelProvider || "未知"}</dd></div>
        <div><dt>{sessionMessages.sourceColumn}</dt><dd>{summary.source}</dd></div>
        <div><dt>{sessionMessages.createdAt}</dt><dd>{formatTimestamp(summary.createdAt)}</dd></div>
        <div><dt>{sessionMessages.updatedAt}</dt><dd>{formatTimestamp(summary.updatedAt)}</dd></div>
      </dl>
      <MutationGate mutation={mutation} />
      {mutationSummary && <p className="session-mutation-summary" role="status">{mutationSummary}</p>}
      <MutationOutcomeList outcomes={mutationOutcomes} />
      {exportState === "saved" && <p className="session-export-status" role="status">{sessionMessages.exportComplete}</p>}
      {exportState === "error" && <p className="inline-error" role="alert">{sessionMessages.exportFailed}</p>}
      {state.kind === "loading" && (
        <p className="session-list-state" role="status"><LoaderCircle className="is-spinning" size={18} aria-hidden="true" />{sessionMessages.loading}</p>
      )}
      {state.kind === "error" && (
        <div className="session-inline-error" role="alert">
          <span>{sessionMessages.detailFailed}</span>
          <button className="secondary-button compact" type="button" onClick={() => onRetry(state.summary)}>
            <RefreshCw size={15} aria-hidden="true" />{sessionMessages.retry}
          </button>
        </div>
      )}
      {state.kind === "loaded" && (
        <div className="session-transcript" aria-label="会话内容">
          {state.detail.entries.map((entry) => entry.kind === "tool" ? (
            <details className="session-tool-entry" key={entry.id}>
              <summary>{entry.label}</summary>
              <pre>{entry.content}</pre>
              {entry.output && <><h3>{sessionMessages.toolOutput}</h3><pre>{entry.output}</pre></>}
            </details>
          ) : (
            <article className={`session-message ${entry.kind}`} key={entry.id}>
              <h2>{entry.label}</h2>
              <p>{entry.content}</p>
            </article>
          ))}
        </div>
      )}
    </>
  );
}

function MutationGate({ mutation }: { mutation: SessionMutationAvailability }) {
  if (mutation.status === "allowed") return null;
  return (
    <p
      className="session-mutation-gate"
      role="status"
      aria-label={sessionMessages.mutationBlockedLabel}
    >
      <ShieldAlert size={17} aria-hidden="true" />
      {sessionMessages.mutationBlocked.unavailable}
    </p>
  );
}

function MutationOutcomeList({ outcomes }: { outcomes: MutationOutcome[] }) {
  if (outcomes.length === 0) return null;
  return (
    <ul className="session-mutation-outcomes" aria-label={sessionMessages.mutationOutcomesLabel}>
      {outcomes.map((outcome) => (
        <li key={outcome.sessionId}>
          {outcome.title}：{sessionMessages.actualState[outcome.actualState]}
        </li>
      ))}
    </ul>
  );
}

function DeleteSessionDialog({
  detail,
  targets,
  knownDescendants: descendants,
  busy,
  failure,
  exportState,
  onCancel,
  onExport,
  onConfirm,
}: {
  detail: SessionDetail | null;
  targets: SessionSummary[];
  knownDescendants: SessionSummary[];
  busy: boolean;
  failure: string | null;
  exportState: "idle" | "exporting" | "saved" | "error";
  onCancel: () => void;
  onExport?: () => void;
  onConfirm: () => void;
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const cancelAction = useRef(onCancel);

  useEffect(() => {
    cancelAction.current = onCancel;
  }, [onCancel]);

  useEffect(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = dialogRef.current;
    if (!dialog) return undefined;
    const focusable = () => Array.from(dialog.querySelectorAll<HTMLElement>(
      "button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex='-1'])",
    ));
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        cancelAction.current();
        return;
      }
      if (event.key !== "Tab") return;
      const controls = focusable();
      if (controls.length === 0) return;
      const first = controls[0];
      const last = controls[controls.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    dialog.addEventListener("keydown", handleKeyDown);
    cancelRef.current?.focus();
    return () => {
      dialog.removeEventListener("keydown", handleKeyDown);
      previousFocus?.focus();
    };
  }, []);

  const isBulk = targets.length > 1 || detail === null;
  const title = isBulk ? sessionMessages.deletionSelectedTitle : sessionMessages.deletionTitle;

  return (
    <div className="dialog-backdrop">
      <section
        ref={dialogRef}
        className="confirmation-dialog session-delete-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="session-delete-title"
        tabIndex={-1}
      >
        <div className="dialog-heading">
          <Trash2 size={22} aria-hidden="true" />
          <div>
            <h2 id="session-delete-title">{title}</h2>
            <p>{isBulk ? sessionMessages.deletionSelectedCount(targets.length) : detail?.title}</p>
          </div>
        </div>
        {detail ? (
          <dl className="session-delete-target">
            <div><dt>{sessionMessages.projectColumn}</dt><dd>{detail.project}</dd></div>
          </dl>
        ) : (
          <ul className="session-delete-targets">
            {targets.map((target) => <li key={target.id}>{target.title}</li>)}
          </ul>
        )}
        <p className="session-delete-warning">{sessionMessages.deletionIrreversible}</p>
        {descendants.length > 0 && (
          <div className="session-delete-descendants">
            <p>{sessionMessages.deletionKnownDescendants(descendants.length)}</p>
            <ul>
              {descendants.map((session) => <li key={session.id}>{session.title}</li>)}
            </ul>
          </div>
        )}
        <p>{sessionMessages.deletionDescendantsUnknown}</p>
        {exportState === "saved" && <p role="status">{sessionMessages.exportComplete}</p>}
        {exportState === "error" && <p className="inline-error" role="alert">{sessionMessages.exportFailed}</p>}
        {failure && <p className="inline-error" role="alert">{failure}</p>}
        <div className="dialog-actions">
          <button ref={cancelRef} className="secondary-button" type="button" onClick={onCancel} disabled={busy}>
            {sessionMessages.cancelDelete}
          </button>
          {detail && onExport && (
            <button
              className="secondary-button"
              type="button"
              onClick={onExport}
              disabled={busy || exportState === "exporting"}
            >
              {exportState === "exporting"
                ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
                : <Download size={16} aria-hidden="true" />}
              {sessionMessages.exportBeforeDelete}
            </button>
          )}
          <button className="danger-button" type="button" onClick={onConfirm} disabled={busy}>
            {busy
              ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
              : <Trash2 size={16} aria-hidden="true" />}
            {isBulk ? sessionMessages.confirmDeleteSelected(targets.length) : sessionMessages.delete}
          </button>
        </div>
      </section>
    </div>
  );
}

function UnavailableState({ availability, onRetry }: { availability: SessionAvailability; onRetry: () => void }) {
  const copy = sessionMessages.unavailable[availability.status as Exclude<SessionAvailabilityStatus, "available">];
  return (
    <section className="blocked-state" role="alert">
      <ShieldAlert size={26} aria-hidden="true" />
      <div>
        <h2>{copy.title}</h2>
        <p>{copy.body}</p>
        {availability.codexVersion && <p className="secondary-note">{availability.codexVersion}</p>}
        <button className="command-button" type="button" onClick={onRetry}>
          <RefreshCw size={17} aria-hidden="true" />
          {sessionMessages.retryCheck}
        </button>
      </div>
    </section>
  );
}

function rememberFacets(
  sessions: SessionSummary[],
  setProjects: React.Dispatch<React.SetStateAction<string[]>>,
  setProviders: React.Dispatch<React.SetStateAction<string[]>>,
) {
  setProjects((current) => sortedUnique([...current, ...sessions.map((session) => session.project)]));
  setProviders((current) => sortedUnique([...current, ...sessions.map((session) => session.modelProvider).filter(Boolean)]));
}

function uniqueSessions(sessions: SessionSummary[]): SessionSummary[] {
  return [...new Map(sessions.map((session) => [session.id, session])).values()];
}

function stateBelongsToTab(state: SessionMutationResult["actualState"], tab: SessionTab): boolean {
  if (state === "unknown") return true;
  return tab === "active" ? state === "active" : state === "archived";
}

function toggleSelection(current: Set<string>, sessionId: string): Set<string> {
  const next = new Set(current);
  if (next.has(sessionId)) next.delete(sessionId);
  else next.add(sessionId);
  return next;
}

function knownDescendants(sessionId: string, sessions: SessionSummary[]): SessionSummary[] {
  const descendants: SessionSummary[] = [];
  const pending = [sessionId];
  const seen = new Set(pending);
  while (pending.length > 0) {
    const parentId = pending.shift();
    for (const session of sessions) {
      const candidateParent = session.parentThreadId ?? session.forkedFromId;
      if (candidateParent !== parentId || seen.has(session.id)) continue;
      seen.add(session.id);
      pending.push(session.id);
      descendants.push(session);
    }
  }
  return descendants;
}

function sortedUnique(values: string[]): string[] {
  return [...new Set(values)].sort((left, right) => left.localeCompare(right, "zh-CN"));
}

function createLeaseId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random()}`;
}

function epochDateTime(value: number): string {
  return new Date(value * 1000).toISOString();
}

function formatTimestamp(value: number): string {
  return new Intl.DateTimeFormat("zh-CN", {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(value * 1000));
}

function failureMessage(failure: SessionFailure | null): string {
  return failure?.category === "recovery_failed"
    ? sessionMessages.unavailable.recovery_failed.body
    : sessionMessages.listFailed;
}
