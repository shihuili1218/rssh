import {
  sessionCommandKey,
  sessionCommandKeyPrefix,
  type SessionInstanceRef,
} from "./session-identity.ts";
import type { AiSettings, CommandKind } from "./types.ts";

/** Fail closed for missing settings/kind. This is shared by the event-time
 * snapshot and the dialog's later revocation check so their policy cannot drift. */
export function isAutoApprovalAllowed(
  settings: AiSettings | null,
  kind?: CommandKind,
): boolean {
  if (!settings || !settings.danger_mode || !kind) return false;
  switch (kind) {
    case "run_command": return settings.auto_run_command;
    case "match_file": return settings.auto_match_file;
    case "download_file": return settings.auto_download_file;
    case "analyze_locally": return settings.auto_analyze_locally;
    case "patch_cp": return settings.auto_patch_cp;
    case "patch_modify": return settings.auto_patch_modify;
    case "patch_diff": return settings.auto_patch_diff;
    case "patch_mv": return settings.auto_patch_mv;
    default: {
      const exhaustive: never = kind;
      void exhaustive;
      return false;
    }
  }
}

export class CommandApprovalRegistry {
  private readonly acknowledged = new Set<string>();
  private readonly attempted = new Set<string>();
  private readonly eligible = new Map<string, boolean>();

  snapshotEligibility(
    session: SessionInstanceRef,
    commandId: string,
    allowedAtArrival: boolean,
  ): boolean {
    const key = sessionCommandKey(session, commandId);
    const existing = this.eligible.get(key);
    if (existing !== undefined) return existing;
    this.eligible.set(key, allowedAtArrival);
    return allowedAtArrival;
  }

  isEligible(session: SessionInstanceRef, commandId: string): boolean {
    return this.eligible.get(sessionCommandKey(session, commandId)) === true;
  }

  /** Preserve the arrival snapshot only while the current policy still allows
   * it. Revocation happens synchronously, before any reactive auto-run effect. */
  eligibleWhileAllowed(
    session: SessionInstanceRef,
    commandId: string,
    currentlyAllowed: boolean,
  ): boolean {
    const key = sessionCommandKey(session, commandId);
    if (!currentlyAllowed) {
      if (this.eligible.has(key)) this.eligible.set(key, false);
      return false;
    }
    return this.eligible.get(key) === true;
  }

  revokeEligibility(session: SessionInstanceRef, commandId: string): void {
    this.eligible.set(sessionCommandKey(session, commandId), false);
  }

  isAcknowledged(session: SessionInstanceRef, commandId: string): boolean {
    return this.acknowledged.has(sessionCommandKey(session, commandId));
  }

  markAcknowledged(session: SessionInstanceRef, commandId: string): void {
    this.acknowledged.add(sessionCommandKey(session, commandId));
  }

  clearAcknowledged(session: SessionInstanceRef, commandId: string): void {
    this.acknowledged.delete(sessionCommandKey(session, commandId));
  }

  wasAttempted(session: SessionInstanceRef, commandId: string): boolean {
    return this.attempted.has(sessionCommandKey(session, commandId));
  }

  markAttempted(session: SessionInstanceRef, commandId: string): void {
    this.attempted.add(sessionCommandKey(session, commandId));
  }

  clear(session: SessionInstanceRef, commandId: string): void {
    const key = sessionCommandKey(session, commandId);
    this.acknowledged.delete(key);
    this.attempted.delete(key);
    this.eligible.delete(key);
  }

  clearSession(session: SessionInstanceRef): void {
    const prefix = sessionCommandKeyPrefix(session);
    for (const key of this.acknowledged) {
      if (key.startsWith(prefix)) this.acknowledged.delete(key);
    }
    for (const key of this.attempted) {
      if (key.startsWith(prefix)) this.attempted.delete(key);
    }
    for (const key of this.eligible.keys()) {
      if (key.startsWith(prefix)) this.eligible.delete(key);
    }
  }
}

export const commandApprovals = new CommandApprovalRegistry();
