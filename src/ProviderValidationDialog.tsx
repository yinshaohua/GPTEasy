import { useEffect, useState } from "react";
import { Check, Circle, LoaderCircle, X } from "lucide-react";

import type { ProviderFailure, ProviderValidationStage } from "./contracts/provider";
import { providerFailureMessages, providerMessages } from "./messages";

export type ProviderValidationSource =
  | { kind: "detail"; providerName: string }
  | { kind: "catalog"; providerName: string };

export interface ProviderValidationSession {
  source: ProviderValidationSource;
  status: "running" | "succeeded" | "failed";
  stage: ProviderValidationStage;
  stageStartedAt: number;
  failure: ProviderFailure | null;
}

interface ProviderValidationDialogProps {
  session: ProviderValidationSession;
  onCancel: () => void;
  onClose: () => void;
}

const STAGES: Array<{ id: ProviderValidationStage; label: string }> = [
  { id: "models_confirmed", label: providerMessages.modelsConfirmed },
  { id: "responses_stream", label: providerMessages.responsesStream },
  { id: "tool_round_trip", label: providerMessages.toolRoundTrip },
];
const WAITING_THRESHOLDS: Record<ProviderValidationStage, number> = {
  models_confirmed: 10,
  responses_stream: 30,
  tool_round_trip: 30,
};

export default function ProviderValidationDialog({
  session,
  onCancel,
  onClose,
}: ProviderValidationDialogProps) {
  const [now, setNow] = useState(Date.now());
  const [reduceMotion, setReduceMotion] = useState(false);

  useEffect(() => {
    const media = window.matchMedia?.("(prefers-reduced-motion: reduce)");
    if (!media) return;
    const updatePreference = () => setReduceMotion(media.matches);
    updatePreference();
    media.addEventListener?.("change", updatePreference);
    return () => media.removeEventListener?.("change", updatePreference);
  }, []);

  useEffect(() => {
    if (session.status !== "running") return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), 1_000);
    return () => window.clearInterval(timer);
  }, [session.stage, session.status, session.stageStartedAt]);

  const activeIndex = STAGES.findIndex((stage) => stage.id === session.stage);
  const elapsedSeconds = Math.max(0, Math.floor((now - session.stageStartedAt) / 1_000));
  const succeeded = session.status === "succeeded";
  const failed = session.status === "failed";
  const closeLabel = session.source.kind === "detail" && failed
    ? providerMessages.returnToEdit
    : providerMessages.finishValidation;

  return (
    <div className="dialog-backdrop validation-dialog-backdrop">
      <section
        className="validation-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="provider-validation-dialog-title"
        onKeyDown={(event) => {
          if (event.key === "Escape" || event.key === "Tab") {
            event.preventDefault();
            event.stopPropagation();
            event.currentTarget.querySelector<HTMLButtonElement>("button")?.focus();
          }
        }}
      >
        <header className="validation-dialog-header">
          <div>
            <h2 id="provider-validation-dialog-title">{providerMessages.validationTitle}</h2>
            <p>{session.source.providerName || providerMessages.unnamedProvider}</p>
          </div>
          <strong className={`validation-result is-${session.status}`} role="status">
            {session.status === "running"
              ? providerMessages.validationRunning
              : succeeded
                ? providerMessages.validationSucceeded
                : providerMessages.validationFailed}
          </strong>
        </header>

        <ol className="validation-dialog-steps" aria-label={providerMessages.validationProgress}>
          {STAGES.map((stage, index) => {
            const state = succeeded
              ? "complete"
              : failed && index === activeIndex
                ? "failed"
                : index < activeIndex
                  ? "complete"
                  : session.status === "running" && index === activeIndex
                    ? "active"
                    : "pending";
            return (
              <li
                className={`validation-dialog-step is-${state}`}
                key={stage.id}
                aria-current={state === "active" ? "step" : undefined}
              >
                <span className="validation-step-icon" aria-hidden="true">
                  {state === "complete" ? (
                    <Check size={18} />
                  ) : state === "failed" ? (
                    <X size={18} />
                  ) : state === "active" && !reduceMotion ? (
                    <LoaderCircle className="is-spinning" size={18} />
                  ) : (
                    <Circle size={18} />
                  )}
                </span>
                <span className="validation-step-copy">
                  <strong>{stage.label}</strong>
                  <span className="validation-step-state">
                    {state === "complete"
                      ? providerMessages.stepComplete
                      : state === "failed"
                        ? providerMessages.stepFailed
                        : state === "active"
                          ? `${providerMessages.stepRunning} · 已用 ${elapsedSeconds} 秒`
                          : providerMessages.stepPending}
                  </span>
                  {state === "active" && elapsedSeconds > WAITING_THRESHOLDS[stage.id] && (
                    <span className="validation-waiting">{providerMessages.stillWaiting}</span>
                  )}
                </span>
              </li>
            );
          })}
        </ol>

        {session.failure && (
          <div className="validation-error" role="alert">
            <p>
              {providerFailureMessages[session.failure.messageId] ??
                providerMessages.validationFallback}
            </p>
            <details>
              <summary>{providerMessages.technicalDetails}</summary>
              <code>{session.failure.category} · {session.failure.messageId}</code>
            </details>
          </div>
        )}

        <div className="dialog-actions validation-dialog-actions">
          {session.status === "running" ? (
            <button className="danger-button" type="button" onClick={onCancel} autoFocus>
              <X size={17} aria-hidden="true" />
              {providerMessages.cancelValidation}
            </button>
          ) : (
            <button className={failed ? "secondary-button" : "command-button"} type="button" onClick={onClose} autoFocus>
              {closeLabel}
            </button>
          )}
        </div>
      </section>
    </div>
  );
}
