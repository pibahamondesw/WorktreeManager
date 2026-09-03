interface Bounds {
  top: number;
  bottom: number;
}

/** Matches the `p-6` padding of the worktree list scroll container. */
export const CARD_SCROLL_PADDING = 24;

/**
 * How much to scroll so the card is visible, aligning it near the top of the
 * container when it is off-screen and leaving an already visible card in place.
 */
export function cardScrollDelta(
  container: Bounds,
  card: Bounds,
  padding = CARD_SCROLL_PADDING
): number {
  if (card.top >= container.top && card.bottom <= container.bottom) return 0;
  return card.top - container.top - padding;
}
