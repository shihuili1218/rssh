/**
 * Compact token-count formatting for the chat panel toolbar.
 * Space there is tight — "1234567" becomes "1.2M", "45678" becomes "45.7k".
 */
export function formatTokenCount(n: number): string {
  if (n < 1000) return String(n);
  const compact = n < 1_000_000 ? [n / 1000, "k"] as const : [n / 1_000_000, "M"] as const;
  const [v, unit] = compact;
  // One decimal, but "12.0k" reads worse than "12k".
  const s = v >= 100 ? String(Math.round(v)) : v.toFixed(1).replace(/\.0$/, "");
  return s + unit;
}
