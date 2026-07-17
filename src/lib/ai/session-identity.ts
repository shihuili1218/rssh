export interface SessionInstanceRef {
  readonly tabId: string;
  readonly instanceId: string;
}

export function sessionCommandKeyPrefix(session: SessionInstanceRef): string {
  // sessionCommandKey serializes one additional array element. Removing this
  // array's closing bracket yields an escape-safe prefix for that element.
  return `${JSON.stringify([session.tabId, session.instanceId]).slice(0, -1)},`;
}

/** A single provider tool call can produce multiple sequential command cards
 * (patch_file has cp/modify/diff/mv). The proposal/card id, not tool_call_id,
 * owns frontend execution and approval state. Encode it with the actor identity
 * so cards from other tabs or replacement actors cannot collide either. */
export function sessionCommandKey(session: SessionInstanceRef, commandId: string): string {
  return JSON.stringify([session.tabId, session.instanceId, commandId]);
}
