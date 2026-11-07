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

/** The characters of short names, from `orisu_mangle::names::ALPHABET`; the first 52 can start one. */
export const NAME_ALPHABET = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
export const NAME_FIRST = 52;
