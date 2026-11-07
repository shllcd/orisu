import fs from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";
import * as native from "./native.ts";
import { rewriteUrls } from "./urls.ts";

export {
  check,
  expandVariantGroups,
  Features,
  loadManglePlan,
  lsp,
  mangleModule,
  Polyfills,
  Scanner,
  type Compiler,
  type Diagnostic,
  type DiagnosticCode,
  type MangledModule,
  type ManglePlan,
} from "./native.ts";
export { normalizePath } from "./urls.ts";

export type Resolver = (id: string, base: string) => Promise<string | false | undefined>;

export interface CompileOptions {
  base: string;
  onDependency?: (path: string) => void;
  shouldRewriteUrls?: boolean;
  polyfills?: number;
  customCssResolver?: Resolver;
}

/** Node-style CSS resolution: relative files, then packages in `node_modules` and `NODE_PATH` via `exports` (style conditions),
 * `style`, `main` and `index.css`. */
export const resolveCss = (id: string, base: string): string | undefined => native.resolveCss(id, base) ?? undefined;

const loadStylesheet = async (id: string, base: string, options: CompileOptions): Promise<native.LoadedStylesheet> => {
  const resolved = (await options.customCssResolver?.(id, base)) || resolveCss(id, base);
  if (!resolved) throw new Error(`Could not resolve '${id}' from '${base}'`);
  options.onDependency?.(resolved);
  const sheetBase = path.dirname(resolved);
  let content = await fs.readFile(resolved, "utf8");
  if (options.shouldRewriteUrls) content = rewriteUrls(content, sheetBase, options.base);
  return { path: resolved, base: sheetBase, content };
};

const ensureSourceRootExists = async (compiler: native.Compiler): Promise<void> => {
  const root = compiler.root;
  if (root === null || root === "none") return;
  const parts: string[] = [];
  for (const part of root.pattern.split("/")) {
    if (/[*{]/.test(part)) break;
    parts.push(part);
  }
  const exists = await fs
    .stat(path.resolve(root.base, parts.join("/")))
    .then((stat) => stat.isDirectory())
    .catch(() => false);
  if (!exists) throw new Error(`The \`source(${root.pattern})\` does not exist or is not a directory.`);
};

/** Compiles a stylesheet, resolving `@import`s like `@tailwindcss/node`. */
export const compile = async (css: string, options: CompileOptions): Promise<native.Compiler> => {
  const compiler = await native.compileAsync(
    css,
    { base: options.base, polyfills: options.polyfills },
    (id, base) => loadStylesheet(id, base, options),
  );
  await ensureSourceRootExists(compiler);
  return compiler;
};

export interface OptimizeOptions {
  file?: string;
  minify?: boolean;
}

type LightningCss = typeof import("lightningcss");

let lightningcss: Promise<LightningCss | null> | undefined;

/** Lowers and minifies CSS with Lightning CSS (same settings as Tailwind), or returns it untouched when missing. */
/** The scanner sources of a compiled stylesheet: its `source(…)` root (`null` scans `base`, `"none"` nothing), then its `@source` entries. */
export const scanSources = (compiler: native.Compiler, base: string): native.SourceEntry[] => {
  const root = compiler.root;
  const sources = root === "none" ? [] : [root === null ? { base, pattern: "**/*", negated: false } : { ...root, negated: false }];
  return sources.concat(compiler.sources);
};

export const optimize = async (input: string, { file = "input.css", minify = false }: OptimizeOptions = {}) => {
  lightningcss ??= import("lightningcss").catch(() =>
    import(pathToFileURL(createRequire(path.join(process.cwd(), "package.json")).resolve("lightningcss")).href).then(
      (mod: LightningCss & { default?: LightningCss }) => mod.default ?? mod,
      () => null,
    ),
  );
  const lcss = await lightningcss;
  if (!lcss) return input;
  const run = (code: Uint8Array) =>
    lcss.transform({
      filename: file,
      code,
      minify,
      drafts: { customMedia: true },
      nonStandard: { deepSelectorCombinator: true },
      include: lcss.Features.Nesting | lcss.Features.MediaQueries,
      exclude: lcss.Features.LogicalProperties | lcss.Features.DirSelector | lcss.Features.LightDark,
      targets: { safari: (16 << 16) | (4 << 8), ios_saf: (16 << 16) | (4 << 8), firefox: 128 << 16, chrome: 111 << 16 },
      errorRecovery: true,
    });
  const first = run(Buffer.from(input));
  const warnings = first.warnings.filter(
    (warning) =>
      !/'(deep|slotted|global)' is not recognized as a valid pseudo-/.test(warning.message) &&
      !/Unknown at rule: @position-try/.test(warning.message),
  );
  if (process.env.NODE_ENV !== "test" && warnings.length > 0) {
    const lines = input.split("\n");
    const output = [`Found ${warnings.length} ${warnings.length === 1 ? "warning" : "warnings"} while optimizing generated CSS:`];
    for (const [idx, warning] of warnings.entries()) {
      output.push("");
      if (warnings.length > 1) output.push(`Issue #${idx + 1}:`);
      const start = Math.max(0, warning.loc.line - 3);
      const end = Math.min(lines.length, warning.loc.line + 2);
      const snippet = lines
        .slice(start, end)
        .map((line, i) => (start + i + 1 === warning.loc.line ? `\x1B[2m│\x1B[22m ${line}` : `\x1B[2m│ ${line}\x1B[22m`));
      snippet.splice(
        warning.loc.line - start,
        0,
        `\x1B[2m┆\x1B[22m${" ".repeat(warning.loc.column - 1)} \x1B[33m\x1B[2m^--\x1B[22m ${warning.message}\x1B[39m`,
        "\x1B[2m┆\x1B[22m",
      );
      output.push(...snippet);
    }
    output.push("");
    console.warn(output.join("\n"));
  }
  return run(first.code).code.toString().replaceAll("@media not (", "@media not all and (");
};
