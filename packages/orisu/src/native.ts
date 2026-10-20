import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import type { DiagnosticCode } from "./codes.ts";

export type { DiagnosticCode } from "./codes.ts";

export interface ChangedContent {
  file?: string;
  content?: string;
  extension: string;
}

export interface GlobEntry {
  base: string;
  pattern: string;
}

export interface SourceEntry {
  base: string;
  pattern: string;
  negated: boolean;
}

export interface CandidateWithPosition {
  candidate: string;
  position: number;
}

export interface Scanner {
  scan(): string[];
  scanFiles(input: ChangedContent[]): string[];
  getCandidatesWithPositions(input: ChangedContent): CandidateWithPosition[];
  readonly files: string[];
  readonly scannedFiles: string[];
  readonly globs: GlobEntry[];
  readonly normalizedSources: GlobEntry[];
}

export interface LoadedStylesheet {
  path: string;
  base: string;
  content: string;
}

export interface ClassEntry {
  name: string;
  modifiers: string[];
}

export interface VariantEntry {
  name: string;
  isArbitrary: boolean;
  values: string[];
  hasDash: boolean;
}

export interface ThemeEntry {
  key: string;
  value: string;
  options: number;
}

export interface MergeTable {
  prefix: string | null;
  keys: string[];
  values: number[];
  erases: number[][];
}

export interface DiagnosticEdit {
  start: number;
  end: number;
  text: string;
}

export interface DiagnosticPosition {
  offset: number;
  line: number;
  column: number;
}

export interface Diagnostic {
  code: DiagnosticCode;
  severity: "error" | "warn";
  message: string;
  file: string;
  start: DiagnosticPosition;
  end: DiagnosticPosition;
  labels: { start: number; end: number; message: string }[];
  fixes: { title: string; edits: DiagnosticEdit[] }[];
  data: Record<string, string>;
  rendered: string;
}

export type CompilerRoot = null | "none" | GlobEntry;

export interface CanonicalizeOptions {
  rem?: number;
  collapse?: boolean;
  logicalToPhysical?: boolean;
}

export interface AttributifyOptions {
  prefix: string;
  prefixedOnly: boolean;
}

export interface Compiler {
  build(candidates: string[]): string;
  readonly features: number;
  readonly sources: SourceEntry[];
  readonly root: CompilerRoot;
  readonly prefix: string | null;
  readonly important: boolean;
  readonly attributify: AttributifyOptions | null;
  readonly invalidCandidates: string[];
  candidatesToCss(classes: string[]): (string | null)[];
  getClassOrder(classes: string[]): (number | null)[];
  getClassList(): ClassEntry[];
  getVariants(): VariantEntry[];
  getVariantSelectors(root: string, value?: string | null, modifier?: string | null): string[];
  printCandidates(classes: string[]): string[][];
  canonicalizeCandidates(candidates: string[], options?: CanonicalizeOptions | null): string[];
  themeEntries(): ThemeEntry[];
  resolveThemeValue(path: string, forceInline?: boolean | null): string | null;
  utilityNames(): string[];
  variantNames(): string[];
  merge(classes: string[]): string;
  mergeTable(classes: string[]): MergeTable;
}

export interface NativeCompileOptions {
  base?: string;
  polyfills?: number;
}

interface NativeCompiler extends Omit<Compiler, "build"> {
  buildChanged(candidates: string[]): string | null;
}

interface Binding {
  Scanner: new (options: { sources?: SourceEntry[]; attributify?: AttributifyOptions | null; cache?: string }) => Scanner;
  compile(
    css: string,
    options?: NativeCompileOptions,
    loadStylesheet?: (id: string, base: string) => LoadedStylesheet,
  ): NativeCompiler;
  expandVariantGroups(code: string): string | null;
  check(entry: string, files: string[], fix?: boolean): string;
  lsp(): void;
}

export const Features = {
  None: 0,
  AtApply: 1 << 0,
  AtImport: 1 << 1,
  ThemeFunction: 1 << 3,
  Utilities: 1 << 4,
  Variants: 1 << 5,
  AtTheme: 1 << 6,
} as const;

export const Polyfills = {
  None: 0,
  AtProperty: 1 << 0,
  ColorMix: 1 << 1,
  All: (1 << 0) | (1 << 1),
} as const;

const BINDINGS: Record<string, string[]> = {
  "darwin-arm64": ["darwin-arm64"],
  "darwin-x64": ["darwin-x64"],
  "linux-x64": ["linux-x64-gnu", "linux-x64-musl"],
  "linux-arm64": ["linux-arm64-gnu", "linux-arm64-musl"],
  "win32-x64": ["win32-x64-msvc"],
  "win32-arm64": ["win32-arm64-msvc"],
};

const load = (): Binding => {
  const require = createRequire(import.meta.url);
  const override = process.env.ORISU_NATIVE_PATH;
  if (override) return require(override) as Binding;
  const target = `${process.platform}-${process.arch}`;
  const local = fileURLToPath(new URL(`../orisu.${target}.node`, import.meta.url));
  if (existsSync(local)) return require(local) as Binding;
  const errors: unknown[] = [];
  for (const name of BINDINGS[target] ?? []) {
    try {
      return require(`@orisu/binding-${name}`) as Binding;
    } catch (error) {
      errors.push(error);
    }
  }
  throw new Error(`orisu: no native binding for ${target}; install \`@orisu/binding-${BINDINGS[target]?.[0] ?? target}\``, {
    cause: errors[0],
  });
};

const binding = load();

/** Keeps the last CSS on the JS side, so an unchanged build costs no copy of the stylesheet. */
const cachingBuild = (compiler: NativeCompiler): Compiler => {
  let css = "";
  return Object.assign(compiler, { build: (candidates: string[]): string => (css = compiler.buildChanged(candidates) ?? css) });
};

export const Scanner = binding.Scanner;
/** Lints `files` against the design system of `entry`; with `fix`, writes the fixes back. */
export const check = (entry: string, files: string[], fix = false): Diagnostic[] =>
  binding
    .check(entry, files, fix)
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => JSON.parse(line) as Diagnostic);
/** Serves the language server over stdio until the client exits. */
export const lsp = binding.lsp;
/** Expands `hover:(a b)` variant groups inside the string literals of `code`; `null` when none were found. */
export const expandVariantGroups = binding.expandVariantGroups;
/** Compiles synchronously; `@import` needs {@link compileAsync}. */
export const compile = (css: string, options?: NativeCompileOptions): Compiler => cachingBuild(binding.compile(css, options));

/**
 * Compiles with asynchronously loaded `@import`s: the binding asks for them synchronously,
 * so every unknown one aborts the attempt, gets loaded, and compilation restarts.
 */
export const compileAsync = async (
  css: string,
  options: NativeCompileOptions,
  load: (id: string, base: string) => Promise<LoadedStylesheet>,
): Promise<Compiler> => {
  const sheets = new Map<string, LoadedStylesheet>();
  for (;;) {
    const missing: [string, string][] = [];
    try {
      const loadSheet = (id: string, base: string): LoadedStylesheet => {
        const sheet = sheets.get(id + "\0" + base);
        if (sheet) return sheet;
        missing.push([id, base]);
        throw new Error(`pending '${id}'`);
      };
      return cachingBuild(binding.compile(css, options, loadSheet));
    } catch (error) {
      if (missing.length === 0) throw error;
      await Promise.all(missing.map(async ([id, base]) => sheets.set(id + "\0" + base, await load(id, base))));
    }
  }
};
