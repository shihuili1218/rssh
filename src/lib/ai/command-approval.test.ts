import { describe, expect, it } from "vitest";
import { CommandApprovalRegistry } from "./command-approval.ts";

describe("CommandApprovalRegistry", () => {
  it("isolates acknowledgement and approval attempts by session instance", () => {
    const registry = new CommandApprovalRegistry();
    const sessionA = { tabId: "tab-a", instanceId: "instance-a" };
    const sessionB = { tabId: "tab-b", instanceId: "instance-b" };

    registry.markAcknowledged(sessionA, "call-0");
    registry.markAttempted(sessionA, "call-0");

    expect(registry.isAcknowledged(sessionA, "call-0")).toBe(true);
    expect(registry.wasAttempted(sessionA, "call-0")).toBe(true);
    expect(registry.isAcknowledged(sessionB, "call-0")).toBe(false);
    expect(registry.wasAttempted(sessionB, "call-0")).toBe(false);
  });

  it("does not grant automatic approval when settings become permissive later", () => {
    const registry = new CommandApprovalRegistry();
    const session = { tabId: "tab-a", instanceId: "instance-a" };

    expect(registry.snapshotEligibility(session, "call-0", false)).toBe(false);
    expect(registry.snapshotEligibility(session, "call-0", true)).toBe(false);
    expect(registry.isEligible(session, "call-0")).toBe(false);
  });

  it("keeps sequential cards from one provider tool call independent", () => {
    const registry = new CommandApprovalRegistry();
    const session = { tabId: "tab-a", instanceId: "instance-a" };

    registry.snapshotEligibility(session, "patch-cp-card", true);
    registry.markAttempted(session, "patch-cp-card");
    registry.markAcknowledged(session, "patch-cp-card");

    expect(registry.snapshotEligibility(session, "patch-modify-card", true)).toBe(true);
    expect(registry.wasAttempted(session, "patch-modify-card")).toBe(false);
    expect(registry.isAcknowledged(session, "patch-modify-card")).toBe(false);
  });

  it("revokes a captured automatic approval when settings become restrictive", () => {
    const registry = new CommandApprovalRegistry();
    const session = { tabId: "tab-a", instanceId: "instance-a" };

    expect(registry.snapshotEligibility(session, "call-0", true)).toBe(true);
    registry.revokeEligibility(session, "call-0");
    expect(registry.isEligible(session, "call-0")).toBe(false);
    expect(registry.snapshotEligibility(session, "call-0", true)).toBe(false);
  });

  it("fails closed synchronously when a mounted dialog sees restrictive settings", () => {
    const registry = new CommandApprovalRegistry();
    const session = { tabId: "tab-a", instanceId: "instance-a" };
    registry.snapshotEligibility(session, "call-0", true);

    expect(registry.eligibleWhileAllowed(session, "call-0", false)).toBe(false);
    expect(registry.eligibleWhileAllowed(session, "call-0", true)).toBe(false);
  });

  it("clears only the requested session instance", () => {
    const registry = new CommandApprovalRegistry();
    const oldSession = { tabId: "tab-a", instanceId: "instance-old" };
    const liveSession = { tabId: "tab-a", instanceId: "instance-live" };
    for (const session of [oldSession, liveSession]) {
      registry.snapshotEligibility(session, "call-0", true);
      registry.markAcknowledged(session, "call-0");
      registry.markAttempted(session, "call-0");
    }

    registry.clearSession(oldSession);

    expect(registry.isEligible(oldSession, "call-0")).toBe(false);
    expect(registry.isAcknowledged(oldSession, "call-0")).toBe(false);
    expect(registry.wasAttempted(oldSession, "call-0")).toBe(false);
    expect(registry.isEligible(liveSession, "call-0")).toBe(true);
    expect(registry.isAcknowledged(liveSession, "call-0")).toBe(true);
    expect(registry.wasAttempted(liveSession, "call-0")).toBe(true);
  });
});
