import { existsSync, readFileSync, statSync } from "node:fs";
import fs from "node:fs/promises";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";
import * as native from "./native.ts";
import { rewriteUrls } from "./urls.ts";

export { check, expandVariantGroups, Features, lsp, Polyfills, Scanner, type Compiler, type Diagnostic, type DiagnosticCode } from "./native.ts";
export { normalizePath } from "./urls.ts";

export type Resolver = (id: string, base: string) => Promise<string | false | undefined>;

export interface CompileOptions {
  base: string;
  onDependency?: (path: string) => void;
  shouldRewriteUrls?: boolean;
  polyfills?: number;
  customCssResolver?: Resolver;
}

const isFile = (file: string): boolean => {
  try {
    return statSync(file).isFile();
  } catch {
    return false;
  }
};

const STYLE_CONDITIONS = ["style", "development", "production", "import", "default"];

const pickExport = (target: unknown): string | undefined => {
  if (typeof target === "string") return target;
  if (Array.isArray(target)) {
    for (const item of target) {
      const picked = pickExport(item);
      if (picked) return picked;
    }
    return undefined;
  }
  if (target && typeof target === "object") {
    for (const [condition, value] of Object.entries(target)) {
      if (!STYLE_CONDITIONS.includes(condition)) continue;
      const picked = pickExport(value);
      if (picked) return picked;
    }
  }
  return undefined;
};

const resolvePackage = (dir: string, subpath: string): string | undefined => {
  const manifest = path.join(dir, "package.json");
  if (!existsSync(manifest)) {
    const file = path.join(dir, subpath);
    return isFile(file) ? file : isFile(file + ".css") ? file + ".css" : undefined;
  }
  const pkg = JSON.parse(readFileSync(manifest, "utf8")) as Record<string, unknown>;
  const key = subpath ? "./" + subpath : ".";
  const exports = pkg.exports as Record<string, unknown> | string | undefined;
  if (exports !== undefined) {
    const map =
      typeof exports === "string" || Array.isArray(exports) || !Object.keys(exports)[0]?.startsWith(".") ? { ".": exports } : exports;
    const target = pickExport(map[key]) ?? pickExport(map[key + ".css"]);
    if (target) return path.join(dir, target);
  }
  if (subpath) {
    const file = path.join(dir, subpath);
    return isFile(file) ? file : isFile(file + ".css") ? file + ".css" : undefined;
  }
  for (const field of [pkg.style, pkg.main]) {
    if (typeof field === "string" && field.endsWith(".css")) return path.join(dir, field);
  }
  const index = path.join(dir, "index.css");
  return isFile(index) ? index : undefined;
};

/** Node-style CSS resolution: relative files, then packages via `exports` (`style` condition), `style` and `main`. */
export const resolveCss = (id: string, base: string): string | undefined => {
  if (id.startsWith(".") || path.isAbsolute(id)) {
    const file = path.resolve(base, id);
    return isFile(file) ? file : isFile(file + ".css") ? file + ".css" : undefined;
  }
  const parts = id.split("/");
  const name = id.startsWith("@") ? parts.slice(0, 2).join("/") : parts[0]!;
  const subpath = parts.slice(id.startsWith("@") ? 2 : 1).join("/");
  const roots: string[] = [];
  for (let dir = path.resolve(base); ; dir = path.dirname(dir)) {
    roots.push(path.join(dir, "node_modules"));
    if (path.dirname(dir) === dir) break;
  }
  roots.push(...(process.env.NODE_PATH ?? "").split(path.delimiter).filter(Boolean));
  for (const root of roots) {
    const dir = path.join(root, name);
    if (!existsSync(dir)) continue;
    const resolved = resolvePackage(dir, subpath);
    if (resolved) return resolved;
  }
  return undefined;
};

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
