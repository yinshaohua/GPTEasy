import { useEffect, useState } from "react";
import { AlertTriangle, CheckCircle2, LoaderCircle } from "lucide-react";

import {
  getSessionVisibilityStatus,
  listenSessionVisibilityStatus,
  type PendingSessionVisibility,
} from "./contracts/session";
import { sessionMessages } from "./messages";

export default function SessionVisibilityStatus() {
  const [status, setStatus] = useState<PendingSessionVisibility | null>(null);

  useEffect(() => {
    let current = true;
    let eventVersion = 0;
    let unlisten: (() => void) | undefined;
    const loadSnapshot = () => {
      const versionBeforeSnapshot = eventVersion;
      void getSessionVisibilityStatus()
        .then((next) => {
          if (current && eventVersion === versionBeforeSnapshot) setStatus(next ?? null);
        })
        .catch(() => undefined);
    };
    void listenSessionVisibilityStatus((next) => {
      eventVersion += 1;
      if (current) setStatus(next);
    }).then((dispose) => {
      if (!current) {
        dispose();
        return;
      }
      unlisten = dispose;
      loadSnapshot();
    }).catch(() => {
      if (current) loadSnapshot();
    });
    return () => {
      current = false;
      unlisten?.();
    };
  }, []);

  if (!status) return null;

  const blocked = status.status === "blocked";
  const running = status.status === "running";
  const message = sessionVisibilityStatusMessage(status);
  return (
    <div
      className={`session-visibility-global-status is-${status.status}`}
      role={blocked ? "alert" : "status"}
      aria-label={sessionMessages.visibility.auto.label}
    >
      {running
        ? <LoaderCircle className="is-spinning" size={16} aria-hidden="true" />
        : blocked
          ? <AlertTriangle size={16} aria-hidden="true" />
          : <CheckCircle2 size={16} aria-hidden="true" />}
      <span>{message}</span>
    </div>
  );
}

function sessionVisibilityStatusMessage(status: PendingSessionVisibility): string {
  switch (status.status) {
    case "running":
      return sessionMessages.visibility.auto.running;
    case "partial":
      return sessionMessages.visibility.auto.partial(status.succeeded, status.retryable);
    case "blocked":
      return sessionMessages.visibility.auto.blocked;
    default:
      return sessionMessages.visibility.auto.pending;
  }
}
