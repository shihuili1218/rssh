interface PrimarySessionWindowDependencies {
  reconcile: () => Promise<unknown>;
  allowResourcePanes: () => void;
  loadAutoOpenLocal: () => Promise<boolean>;
  openLocal: () => void;
}

export async function initializePrimarySessionWindow(
  dependencies: PrimarySessionWindowDependencies,
): Promise<void> {
  try {
    await dependencies.reconcile();
  } catch {
    // Startup reconciliation is best-effort. A backend error must not leave
    // every resource-owning pane permanently unmounted.
  } finally {
    dependencies.allowResourcePanes();
  }

  try {
    if (await dependencies.loadAutoOpenLocal()) {
      dependencies.openLocal();
    }
  } catch {
    // Keep the existing fail-closed auto-open behavior when settings are
    // unavailable; the resource barrier has already been released above.
  }
}
