interface PrimarySessionWindowDependencies {
  signal?: AbortSignal;
  reconcile: () => Promise<unknown>;
  allowResourcePanes: () => void;
  loadAutoOpenLocal: () => Promise<boolean>;
  openLocal: () => void;
}

export async function initializePrimarySessionWindow(
  dependencies: PrimarySessionWindowDependencies,
): Promise<void> {
  if (dependencies.signal?.aborted) return;
  try {
    await dependencies.reconcile();
  } catch {
    // Startup reconciliation is best-effort. A backend error must not leave
    // every resource-owning pane permanently unmounted.
  }
  if (dependencies.signal?.aborted) return;
  dependencies.allowResourcePanes();

  try {
    const autoOpen = await dependencies.loadAutoOpenLocal();
    if (!dependencies.signal?.aborted && autoOpen) {
      dependencies.openLocal();
    }
  } catch {
    // Keep the existing fail-closed auto-open behavior when settings are
    // unavailable; the resource barrier has already been released above.
  }
}
