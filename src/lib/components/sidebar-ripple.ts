/** Vertical-sidebar ripple geometry (desktop Ctrl+Tab cycling).
 *
 *  Both interactions are the same operation — assign each row a width from its
 *  distance to a focal row:
 *    - hover  → cliff falloff (pure CSS, not here)
 *    - cycle  → graded falloff computed by rippleWidth() below
 *
 *  distance = |rowIndex - focusIndex| in the flat nav list. The focused row is
 *  widest; each step narrows by STEP down to a visible FLOOR so even far rows
 *  still show some content. */
export const RIPPLE = { RAIL: 40, FOCUS: 240, STEP: 48, FLOOR: 96 } as const;

export function rippleWidth(distance: number): number {
  return Math.max(RIPPLE.FLOOR, RIPPLE.FOCUS - distance * RIPPLE.STEP);
}
