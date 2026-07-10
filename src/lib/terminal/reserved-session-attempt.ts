export type ReservedSessionOpenResult = {
  kind: "ready";
  sessionId: string;
} | {
  kind: "cancelled";
};

interface ReservedSessionAttemptDependencies {
  makeId: () => string;
  wireEvents: (sessionId: string) => Promise<() => void>;
  close: (sessionId: string) => void | Promise<void>;
}

export function createReservedSessionAttempt(
  dependencies: ReservedSessionAttemptDependencies,
) {
  type CurrentAttempt = {
    sessionId: string;
    pending: boolean;
    disposeEvents?: () => void;
  };

  let generation = 0;
  let current: CurrentAttempt | null = null;
  let destroyed = false;

  async function closeAndIgnore(sessionId: string): Promise<void> {
    try {
      await dependencies.close(sessionId);
    } catch {
      // Cancellation is best-effort and must not revive a stale attempt.
    }
  }

  function requestClose(sessionId: string): void {
    void closeAndIgnore(sessionId);
  }

  function cancel(): void {
    generation += 1;
    const cancelled = current;
    current = null;
    if (!cancelled) return;
    cancelled.disposeEvents?.();
    requestClose(cancelled.sessionId);
  }

  return {
    async open(
      openBackend: (reservedId: string) => Promise<string>,
    ): Promise<ReservedSessionOpenResult> {
      if (destroyed) return { kind: "cancelled" };
      cancel();
      const attemptGeneration = ++generation;
      const reservedId = dependencies.makeId();
      const attempt: CurrentAttempt = {
        sessionId: reservedId,
        pending: true,
      };
      current = attempt;

      let disposeEvents: () => void;
      try {
        disposeEvents = await dependencies.wireEvents(reservedId);
      } catch (error) {
        if (current !== attempt || generation !== attemptGeneration) {
          return { kind: "cancelled" };
        }
        current = null;
        throw error;
      }
      if (current !== attempt || generation !== attemptGeneration) {
        disposeEvents();
        return { kind: "cancelled" };
      }
      attempt.disposeEvents = disposeEvents;

      let sessionId: string;
      try {
        sessionId = await openBackend(reservedId);
      } catch (error) {
        if (current !== attempt || generation !== attemptGeneration) {
          return { kind: "cancelled" };
        }
        current = null;
        disposeEvents();
        // The backend may have activated the reserved id and lost only the IPC
        // response. Closing on every rejection is idempotent for an ordinary
        // open failure and prevents an unreachable ready handle in that case.
        requestClose(reservedId);
        throw error;
      }
      if (current !== attempt || generation !== attemptGeneration) {
        requestClose(sessionId);
        return { kind: "cancelled" };
      }
      if (sessionId !== reservedId) {
        current = null;
        disposeEvents();
        requestClose(reservedId);
        requestClose(sessionId);
        throw new Error("backend returned a different session id");
      }
      attempt.pending = false;
      return { kind: "ready", sessionId };
    },
    cancel,
    destroy(): void {
      destroyed = true;
      cancel();
    },
    isPending(): boolean {
      return current?.pending === true;
    },
    accepts(sessionId: string): boolean {
      return current?.sessionId === sessionId;
    },
  };
}
