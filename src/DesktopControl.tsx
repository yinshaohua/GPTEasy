import { useEffect, useRef, useState } from "react";
import { LoaderCircle, Play, RotateCw } from "lucide-react";

import {
  asDesktopFailure,
  getDesktopSnapshot,
  restartDesktopApplication,
  startDesktopApplication,
  type DesktopSnapshot,
} from "./contracts/desktop";
import { desktopMessages } from "./messages";

type DesktopState =
  | { kind: "loading" }
  | { kind: "loaded"; snapshot: DesktopSnapshot }
  | { kind: "error"; messageId: string };

export default function DesktopControl() {
  const [state, setState] = useState<DesktopState>({ kind: "loading" });
  const [confirmingRestart, setConfirmingRestart] = useState(false);
  const [feedback, setFeedback] = useState<string | null>(null);
  const operationInFlight = useRef(false);

  useEffect(() => {
    let current = true;
    const refresh = () => {
      if (operationInFlight.current) return;
      void getDesktopSnapshot()
        .then((snapshot) => {
          if (current) setState({ kind: "loaded", snapshot });
        })
        .catch((error: unknown) => {
          if (current) setState({ kind: "error", messageId: asDesktopFailure(error).messageId });
        });
    };
    refresh();
    window.addEventListener("focus", refresh);
    return () => {
      current = false;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  const snapshot = state.kind === "loaded" ? state.snapshot : null;
  const busy = state.kind === "loading";
  const action = snapshot?.action ?? "unavailable";
  const statusText = state.kind === "loading"
    ? desktopMessages.checking
    : state.kind === "error"
      ? desktopMessages.unavailable
      : desktopStatusMessage(state.snapshot);
  const actionLabel = action === "restart" ? desktopMessages.restart : desktopMessages.start;

  const start = async () => {
    if (action !== "start") return;
    operationInFlight.current = true;
    setFeedback(null);
    setState({ kind: "loading" });
    try {
      setState({ kind: "loaded", snapshot: await startDesktopApplication() });
      setFeedback(desktopMessages.startSucceeded);
    } catch (error) {
      const failure = asDesktopFailure(error);
      setState({ kind: "error", messageId: failure.messageId });
      setFeedback(desktopFailureMessage(failure.messageId));
    } finally {
      operationInFlight.current = false;
    }
  };

  const restart = async () => {
    if (!snapshot || snapshot.action !== "restart") return;
    operationInFlight.current = true;
    setConfirmingRestart(false);
    setFeedback(null);
    setState({ kind: "loading" });
    try {
      setState({
        kind: "loaded",
        snapshot: await restartDesktopApplication(snapshot.roots),
      });
      setFeedback(desktopMessages.restartSucceeded);
    } catch (error) {
      const failure = asDesktopFailure(error);
      setState({ kind: "error", messageId: failure.messageId });
      setFeedback(desktopFailureMessage(failure.messageId));
    } finally {
      operationInFlight.current = false;
    }
  };

  return (
    <>
      <div className="desktop-control" aria-live="polite">
        <span className={`desktop-status is-${snapshot?.status ?? "unknown"}`}>
          <span className="desktop-status-dot" aria-hidden="true" />
          {statusText}
        </span>
        <button
          className="secondary-button compact"
          type="button"
          disabled={busy || action === "unavailable"}
          onClick={() => {
            if (action === "restart") setConfirmingRestart(true);
            else void start();
          }}
        >
          {busy
            ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
            : action === "restart"
              ? <RotateCw className="button-icon is-orange" size={16} aria-hidden="true" />
              : <Play className="button-icon is-green" size={16} aria-hidden="true" />}
          {actionLabel}
        </button>
      </div>
      {feedback && (
        <p className={state.kind === "error" ? "desktop-feedback is-error" : "desktop-feedback"}
          role={state.kind === "error" ? "alert" : "status"}>
          {feedback}
        </p>
      )}
      {confirmingRestart && (
        <div className="dialog-backdrop">
          <section className="confirmation-dialog" role="dialog" aria-modal="true" aria-labelledby="desktop-restart-title">
            <h2 id="desktop-restart-title">{desktopMessages.restartTitle}</h2>
            <p>{desktopMessages.restartConfirmation}</p>
            <p>{desktopMessages.cliIsolation}</p>
            <div className="dialog-actions">
              <button className="secondary-button" type="button" onClick={() => setConfirmingRestart(false)} autoFocus>
                {desktopMessages.cancel}
              </button>
              <button className="danger-button" type="button" onClick={() => void restart()}>
                {desktopMessages.confirmRestart}
              </button>
            </div>
          </section>
        </div>
      )}
    </>
  );
}

function desktopStatusMessage(snapshot: DesktopSnapshot): string {
  if (snapshot.status === "running") return desktopMessages.running;
  if (snapshot.messageId === "desktop.not_installed") return desktopMessages.notInstalled;
  if (snapshot.status === "stopped") return desktopMessages.stopped;
  return desktopFailureMessage(snapshot.messageId);
}

function desktopFailureMessage(messageId: string): string {
  return desktopMessages.failures[messageId as keyof typeof desktopMessages.failures]
    ?? desktopMessages.failures.state_unavailable;
}
