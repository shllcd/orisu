/** Every orisu diagnostic code, from the catalog in `orisu_diagnostics`. */
export type DiagnosticCode =
  | "UNKNOWN_UTILITY"
  | "UNKNOWN_VARIANT"
  | "INVALID_STYLESHEET"
  | "CIRCULAR_IMPORT"
  | "UNSUPPORTED_PLUGIN"
  | "MANGLE_UNTRACKED_CLASS"
  | "MANGLE_NAME_MERGER"
  | "MANGLE_CLASS_COLLISION"
  | "CONFLICTING_CLASSES"
  | "INVALID_THEME_KEY"
  | "NON_CANONICAL_CLASS";
