/** Variants whose relative order changes the generated selector, from `orisu_merge::ORDER_SENSITIVE`. */
export const ORDER_SENSITIVE: ReadonlySet<string> = new Set([
  "*",
  "**",
  "before",
  "after",
  "backdrop",
  "placeholder",
  "file",
  "marker",
  "selection",
  "first-line",
  "first-letter",
  "details-content",
]);
