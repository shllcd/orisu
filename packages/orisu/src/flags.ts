/** What a compiled stylesheet uses, from `orisu_compiler::compiler::features`. */
export const Features = {
  None: 0,
  AtApply: 1,
  AtImport: 2,
  ThemeFunction: 8,
  Utilities: 16,
  Variants: 32,
  AtTheme: 64,
} as const;

/** Polyfills the compiler emits, from `orisu_optimizer::polyfills`. */
export const Polyfills = {
  None: 0,
  AtProperty: 1,
  ColorMix: 2,
  All: 3,
} as const;
