// Portions ported from @tailwindcss/vite (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

import { readFileSync, realpathSync } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";
import type { Environment, EnvironmentModuleNode, InternalResolveOptions, Plugin, ResolvedConfig, ViteDevServer } from "vite";
import {
  check,
  compile,
  expandVariantGroups,
  Features,
  loadManglePlan,
  mangleModule,
  normalizePath,
  optimize,
  Scanner,
  scanSources,
  type Compiler,
  type MangledModule,
  type ManglePlan,
} from "./index.ts";
import { fillPlaceholders } from "./native.ts";
import { encodeMergeTable } from "./table.ts";

interface LintedFile {
  mtime: number;
  lines: string[];
}

const SPECIAL_QUERY_RE = /[?&](?:worker|sharedworker|raw|url)\b/;
const COMMON_JS_PROXY_RE = /\?commonjs-proxy/;
const INLINE_STYLE_ID_RE = /[?&]index=\d+\.css$/;
const JS_EXTENSIONS_RE = /^\.[cm]?[jt]sx?$/;
const CSS_LANGS_RE = /\.(css|less|sass|scss|styl|stylus|pcss|postcss|sss)(?:$|\?)/;
const MERGE_TABLE_ID = "\0orisu/merge-table";
const MERGE_TABLE_PLACEHOLDER = "__ORISU_MERGE_TABLE__";
const MERGE_MODULE = normalizePath(fileURLToPath(new URL("./merge" + path.extname(fileURLToPath(import.meta.url)), import.meta.url)));
const PACKAGE_DIR = normalizePath(fileURLToPath(new URL("../", import.meta.url)));
const ROOT_WAIT_MS = 2000;
const DIAGNOSTICS_DEBOUNCE_MS = 100;
const SOURCE_FILE_RE = /\.(?:[cm]?[jt]sx?|vue|svelte|astro|mdx)(?:$|\?)/;
const SCRIPT_FILE_RE = /\.[cm]?[jt]sx?$/;
const MANGLE_PLAN_FILE = "mangle.json";

export type PluginOptions = {
  /** Optimize and minify the output CSS. */
  optimize?: boolean | { minify?: boolean };
  /** Expand UnoCSS-style variant groups (`hover:(bg-red-500 text-white)`) in the string literals of source files. */
  variantGroups?: boolean;
  diagnostics?: {
    /** Writes every class diagnostic of the scanned sources as JSON lines to this file, relative to the Vite root. */
    jsonl?: string;
  };
  /**
   * Renames classes to short names in builds: in the stylesheet, in the class positions of the project's scripts
   * (`className`, `cn`, `clsx`, `cva`…) and in the `cn` merge table. A class used anywhere else keeps its name.
   */
  mangle?: boolean | MangleOptions;
};

export type MangleOptions = {
  /** Classes that keep their names; a regular expression keeps every candidate it matches. */
  keep?: (string | RegExp)[];
  /** Starts every short name, so they cannot equal classes of other stylesheets on the page. */
  prefix?: string;
  /** Functions whose arguments are class lists, besides `cn`, `clsx` and the like. */
  classFunctions?: string[];
  /** Attributes and properties whose values are class lists, besides `class`, `className`, `*ClassName` and framework forms like `:class` and `[ngClass]`. */
  classAttributes?: string[];
};

type ResolveFn = (id: string, base: string) => Promise<string | false | undefined>;

function createCustomResolver(
  resolvers: ((id: string, importer: string) => Promise<string | undefined>)[],
  filter: (path: string) => boolean,
): ResolveFn {
  return async (id, base) => {
    const importer = path.resolve(base, "__placeholder__.ts");
    for (const resolver of resolvers) {
      let resolved = await resolver(id, importer);
      if (!resolved || resolved === id) continue;
      if (resolved[0] === ".") resolved = path.resolve(base, resolved);
      if (!filter(resolved) || !path.isAbsolute(resolved)) continue;
      return resolved;
    }
  };
}

type ResolveIdFn = (environment: Environment, id: string, importer?: string, aliasOnly?: boolean) => Promise<string | undefined>;

function createBackCompatIdResolver(config: ResolvedConfig, options?: Partial<InternalResolveOptions>): ResolveIdFn {
  const compatResolve = config.createResolver(options);
  let resolve: ResolveIdFn | undefined;
  return async (environment, id, importer, aliasOnly) => {
    if (environment.name === "client" || environment.name === "ssr") {
      return compatResolve(id, importer, aliasOnly, environment.name === "ssr");
    }
    if (!resolve) {
      const vite: typeof import("vite") = await import(
        pathToFileURL(createRequire(path.join(config.root, "package.json")).resolve("vite")).href
      );
      resolve = vite.createIdResolver(config, options);
    }
    return resolve(environment, id, importer, aliasOnly);
  };
}

function cssResolverFor(env: Environment): ResolveFn {
  const resolve = createBackCompatIdResolver(env.config, {
    ...env.config.resolve,
    extensions: [".css"],
    mainFields: ["style"],
    conditions: ["style", "development|production"],
    tryIndex: false,
    preferRelative: true,
  });
  return createCustomResolver(
    [(id, importer) => resolve(env, id, importer, true), (id, importer) => resolve(env, id, importer, false)],
    (file) => file.endsWith(".css"),
  );
}

export default function orisu(opts: PluginOptions = {}): Plugin[] {
  const servers: ViteDevServer[] = [];
  let config: ResolvedConfig | null = null;
  const rootsByEnv = new DefaultMap<string, Map<string, Root>>(() => new Map());
  const viteProcessedExtensions = new Set<string>();
  let shouldOptimize = true;
  let minify = true;
  let mergeTableKeys = -1;
  let onFirstRoot: (() => void) | undefined;
  const firstRoot = new Promise<void>((resolve) => (onFirstRoot = resolve));
  let diagnosticsTimer: ReturnType<typeof setTimeout> | undefined;
  const diagnosticsByRoot = new WeakMap<Root, { compiler: Compiler; files: Map<string, LintedFile> }>();
  const mangleOptions = opts.mangle ? (opts.mangle === true ? {} : opts.mangle) : null;
  const plans = new Map<string, ManglePlan | null>();
  let savedPlan: ManglePlan | null | undefined;
  const mangledModules = new Map<string, MangledModule>();
  const foreignStylesheets = new Map<string, string>();

  function findRoot(envName: string): Root | undefined {
    for (const name of [envName, "client", ...rootsByEnv.keys()]) {
      for (const root of rootsByEnv.get(name).values()) if (root.compiler) return root;
    }
    return undefined;
  }

  function mergeTable(envName: string): string {
    const plan = mangleOptions && config!.command === "build" ? planFor(envName) : null;
    if (plan) return JSON.stringify(encodeMergeTable(plan.mergeTable()));
    const root = findRoot(envName);
    if (!root) return JSON.stringify(encodeMergeTable({ prefix: null, keys: [], values: [], erases: [] }));
    return JSON.stringify(encodeMergeTable(root.compiler!.mergeTable([...root.candidates])));
  }

  function planPath(): string {
    return path.join(config!.cacheDir, "orisu", MANGLE_PLAN_FILE);
  }

  /** The plan of the stylesheet built in this process, or the one an earlier build of the app saved. */
  function planFor(envName: string): ManglePlan | null {
    const root = findRoot(envName);
    const plan = root ? plans.get(root.inputPath) : plans.values().next().value;
    if (plan !== undefined) return plan;
    if (savedPlan === undefined) {
      try {
        savedPlan = loadManglePlan(readFileSync(planPath(), "utf8"));
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
          throw new Error(`orisu: cannot read the mangle plan \`${planPath()}\`; delete it and build again`, { cause: error });
        }
        savedPlan = null;
      }
    }
    return savedPlan;
  }

  async function mangleStylesheet(root: Root, css: string, env: Environment | undefined): Promise<string> {
    let plan = plans.get(root.inputPath);
    if (plan === undefined) {
      const other = [...plans.keys()][0];
      if (other !== undefined) {
        throw new Error(`[orisu] \`mangle\` renames one stylesheet per build, but both ${other} and ${root.inputPath} use utilities.`);
      }
      const compiler = root.compiler!;
      if (compiler.attributify) {
        config!.logger.warn("[orisu] `mangle` is off: attributify classes are attribute names, which it cannot follow.");
        plan = null;
      } else {
        const keep = mangleOptions!.keep ?? [];
        const patterns = keep.filter((entry) => entry instanceof RegExp);
        const kept = keep.filter((entry) => typeof entry === "string");
        const candidates = [...root.candidates];
        for (const candidate of candidates) if (patterns.some((pattern) => candidate.search(pattern) !== -1)) kept.push(candidate);
        const outDir = path.resolve(config!.root, (env?.config ?? config!).build.outDir);
        const outDirs = [outDir, path.join(realpathSync(config!.root), path.relative(config!.root, outDir))].map((dir) => dir + path.sep);
        const files = root.scannedFiles.filter((file) => !outDirs.some((dir) => file.startsWith(dir)));
        plan = compiler.mangle(
          { files, entry: root.inputPath, candidates, css },
          { ...nativeMangleOptions(), keep: kept },
        );
        await fs.mkdir(path.dirname(planPath()), { recursive: true });
        await fs.writeFile(planPath(), plan.toJson());
      }
      plans.set(root.inputPath, plan);
    }
    return plan ? plan.renameCss(css) : css;
  }

  function nativeMangleOptions() {
    return {
      prefix: mangleOptions!.prefix,
      classFunctions: mangleOptions!.classFunctions,
      classAttributes: mangleOptions!.classAttributes,
      variantGroups: opts.variantGroups === true,
    };
  }

  async function generate(this: { environment: Environment; addWatchFile(file: string): void }, src: string, id: string) {
    if (!isPotentialCssRootFile(id)) return;
    const env = this.environment;
    const roots = rootsByEnv.get(env?.name ?? "default");
    let root = roots.get(id);
    if (!root) {
      root = new Root(id, config!.root, path.join(config!.cacheDir, "orisu"), cssResolverFor(env));
      roots.set(id, root);
    }
    const result = await root.generate(src, (file) => this.addWatchFile(file));
    if (!result) {
      roots.delete(id);
      if (mangleOptions && config!.command === "build" && !/\.module\.css(?:$|\?)/.test(id)) foreignStylesheets.set(id, src);
      return;
    }
    if (mangleOptions && config!.command === "build") result.code = await mangleStylesheet(root, result.code, env);
    onFirstRoot?.();
    onFirstRoot = undefined;
    scheduleDiagnostics();
    return result;
  }

  async function writeDiagnostics() {
    const file = path.resolve(config!.root, opts.diagnostics!.jsonl!);
    const lines: string[] = [];
    for (const roots of rootsByEnv.values()) {
      for (const root of roots.values()) {
        if (!root.compiler) continue;
        const cached = diagnosticsByRoot.get(root);
        const known = cached?.compiler === root.compiler ? cached.files : new Map<string, LintedFile>();
        const files = root.lintedFiles;
        const mtimes = await Promise.all(files.map((file) => fs.stat(file).then((stats) => stats.mtimeMs, () => -1)));
        const linted = new Map<string, LintedFile>();
        const stale: string[] = [];
        files.forEach((file, i) => {
          const entry = known.get(file);
          if (entry?.mtime === mtimes[i]) return linted.set(file, entry);
          linted.set(file, { mtime: mtimes[i]!, lines: [] });
          stale.push(file);
        });
        if (stale.length > 0) {
          for (const diagnostic of check(root.inputPath, stale)) linted.get(diagnostic.file)?.lines.push(JSON.stringify(diagnostic) + "\n");
        }
        diagnosticsByRoot.set(root, { compiler: root.compiler, files: linted });
        for (const entry of linted.values()) lines.push(...entry.lines);
      }
    }
    await fs.mkdir(path.dirname(file), { recursive: true });
    await fs.writeFile(file, [...new Set(lines)].join(""));
  }

  function scheduleDiagnostics() {
    if (!opts.diagnostics?.jsonl) return;
    clearTimeout(diagnosticsTimer);
    diagnosticsTimer = setTimeout(
      () => void writeDiagnostics().catch((error) => config!.logger.error(`[orisu] diagnostics: ${error}`)),
      DIAGNOSTICS_DEBOUNCE_MS,
    );
  }

  function refreshMergeTable(server: ViteDevServer) {
    const root = findRoot("client");
    if (!root || root.candidates.size === mergeTableKeys) return;
    for (const environment of Object.values(server.environments)) {
      const mod = environment.moduleGraph.getModuleById(MERGE_TABLE_ID);
      if (mod) void environment.reloadModule(mod);
    }
  }

  return [
    {
      name: "orisu:scan",
      enforce: "pre",

      config() {
        return {
          optimizeDeps: { exclude: ["orisu"] },
          ssr: { noExternal: ["orisu"] },
        };
      },

      configureServer(server) {
        servers.push(server);
      },

      configResolved(resolved) {
        config = resolved;
        if (opts.optimize !== undefined) shouldOptimize = opts.optimize !== false;
        minify = shouldOptimize && config.build.cssMinify !== false;
        if (typeof opts.optimize === "object") minify = opts.optimize.minify !== false;
      },
    },

    {
      name: "orisu:variant-groups",
      enforce: "pre",
      apply: () => opts.variantGroups === true,
      transform: {
        filter: {
          id: { include: [SOURCE_FILE_RE], exclude: [/\/node_modules\//, /[?&]type=style/, SPECIAL_QUERY_RE] },
          code: ":(",
        },
        handler(code) {
          const expanded = expandVariantGroups(code);
          if (expanded !== null) return { code: expanded, map: null };
        },
      },
    },

    {
      name: "orisu:mangle",
      enforce: "pre",
      apply: (_, env) => mangleOptions !== null && env.command === "build",
      transform: {
        filter: { id: { include: [SCRIPT_FILE_RE], exclude: [/\/node_modules\//] } },
        handler(code, id) {
          const mangled = mangleModule(code, idToPath(id), nativeMangleOptions());
          mangledModules.set(id, mangled);
          if (mangled.code !== null) return { code: mangled.code, map: mangled.map };
        },
      },

      renderChunk(code, chunk) {
        const plan = planFor(this.environment.name);
        if (plan) {
          for (const id of Object.keys(chunk.modules)) {
            const mangled = mangledModules.get(id);
            const error = mangled && plan.checkModule(path.relative(config!.root, idToPath(id)), mangled);
            if (error) this.error(error);
          }
        }
        const filled = plan ? plan.fill(code, chunk.fileName) : fillPlaceholders(code, chunk.fileName);
        if (filled) return { code: filled.code, map: filled.map };
      },

      generateBundle() {
        const plan = planFor(this.environment.name);
        if (!plan) return;
        for (const [id, css] of foreignStylesheets) {
          const error = plan.checkCss(path.relative(config!.root, idToPath(id)), css);
          if (error) this.error(error);
        }
      },
    },

    {
      name: "orisu:merge-table",
      enforce: "pre",

      resolveId: {
        filter: { id: /^#merge-table$/ },
        handler(id, importer) {
          if (importer && normalizePath(importer).startsWith(PACKAGE_DIR)) return { id: MERGE_TABLE_ID, moduleSideEffects: true };
        },
      },

      load: {
        filter: { id: /^\0orisu\/merge-table$/ },
        async handler(id) {
          if (id !== MERGE_TABLE_ID) return;
          const header = `import { registerMergeTable } from ${JSON.stringify(MERGE_MODULE)};\n`;
          if (config!.command === "build") return `${header}registerMergeTable(${MERGE_TABLE_PLACEHOLDER});\n`;
          if (!findRoot(this.environment.name)) {
            await Promise.race([firstRoot, new Promise((resolve) => setTimeout(resolve, ROOT_WAIT_MS))]);
          }
          const root = findRoot(this.environment.name);
          mergeTableKeys = root ? root.candidates.size : -1;
          return `${header}registerMergeTable(${mergeTable(this.environment.name)});\nimport.meta.hot?.accept();\n`;
        },
      },

      renderChunk(code) {
        if (!code.includes(MERGE_TABLE_PLACEHOLDER)) return;
        return { code: code.replace(MERGE_TABLE_PLACEHOLDER, () => mergeTable(this.environment.name)), map: null };
      },
    },

    {
      name: "orisu:generate:serve",
      apply: "serve",
      enforce: "pre",
      transform: {
        filter: {
          id: {
            exclude: [/\/\.vite\//, SPECIAL_QUERY_RE, COMMON_JS_PROXY_RE],
            include: [/\.css(?:\?.*)?$/, /&lang\.css/, INLINE_STYLE_ID_RE],
          },
        },
        async handler(src, id) {
          const result = await generate.call(this, src, id);
          for (const server of servers) refreshMergeTable(server);
          return result;
        },
      },

      hotUpdate({ file, modules, timestamp, server }) {
        const isExternalFile = modules.length > 0 && modules.every((mod) => mod.type === "asset" || mod.id === undefined);
        if (!isExternalFile) return;

        for (const environment of Object.values(server.environments)) {
          if (environment.name === this.environment.name) continue;
          const mods = environment.moduleGraph.getModulesByFile(file);
          if (mods) {
            for (const mod of mods) if (mod.type !== "asset") return;
          }
        }

        const extension = path.extname(file);
        if (JS_EXTENSIONS_RE.test(extension) || CSS_LANGS_RE.test(file)) return;

        if (extension !== "") {
          if (viteProcessedExtensions.has(extension)) return;
          for (const environment of Object.values(server.environments)) {
            for (const mod of environment.moduleGraph.idToModuleMap.values()) {
              if (!mod.file?.endsWith(extension) || mod.type === "asset") continue;
              if (mod.transformResult == null || !mod.id || mod.id.includes("?")) continue;
              viteProcessedExtensions.add(extension);
              return;
            }
          }
        }

        for (const envName of new Set([this.environment.name, "client"])) {
          const roots = rootsByEnv.get(envName);
          if (roots.size === 0 || !isScannedFile(file, modules, roots)) continue;

          const invalidated = new Set<EnvironmentModuleNode>();
          for (const mod of modules) {
            this.environment.moduleGraph.invalidateModule(mod, invalidated, timestamp, true);
          }

          if (envName === this.environment.name) this.environment.hot.send({ type: "full-reload" });
          else server.hot.send({ type: "full-reload" });
          return [];
        }
      },
    },

    {
      name: "orisu:generate:build",
      apply: "build",
      enforce: "pre",
      transform: {
        filter: {
          id: {
            exclude: [/\/\.vite\//, SPECIAL_QUERY_RE, COMMON_JS_PROXY_RE],
            include: [/\.css(?:\?.*)?$/, /&lang\.css/, INLINE_STYLE_ID_RE],
          },
        },
        async handler(src, id) {
          const result = await generate.call(this, src, id);
          if (!result || !shouldOptimize) return result;
          return { code: await optimize(result.code, { minify }), map: undefined };
        },
      },

      async buildEnd() {
        if (!opts.diagnostics?.jsonl) return;
        clearTimeout(diagnosticsTimer);
        await writeDiagnostics();
      },
    },
  ] satisfies Plugin[];
}

function isPotentialCssRootFile(id: string): boolean {
  if (id.includes("/.vite/") || SPECIAL_QUERY_RE.test(id) || COMMON_JS_PROXY_RE.test(id)) return false;
  const extension = path.extname(id.split("?", 2)[0]!).slice(1);
  return extension === "css" || id.includes("&lang.css") || INLINE_STYLE_ID_RE.test(id);
}

function idToPath(id: string): string {
  return path.resolve(id.replace(/\?.*$/, ""));
}

class DefaultMap<K, V> extends Map<K, V> {
  private readonly factory: (key: K) => V;

  constructor(factory: (key: K) => V) {
    super();
    this.factory = factory;
  }

  override get(key: K): V {
    let value = super.get(key);
    if (value === undefined) {
      value = this.factory(key);
      this.set(key, value);
    }
    return value;
  }
}

class Root {
  compiler?: Compiler;
  private scanner?: InstanceType<typeof Scanner>;
  readonly candidates = new Set<string>();
  private files: string[] = [];
  private fileSet = new Set<string>();
  private readonly buildDependencies = new Map<string, number | null>();
  private readonly id: string;
  private readonly base: string;
  private readonly cacheDir: string;
  private readonly cssResolver: ResolveFn;

  constructor(id: string, base: string, cacheDir: string, cssResolver: ResolveFn) {
    this.id = id;
    this.base = base;
    this.cacheDir = cacheDir;
    this.cssResolver = cssResolver;
  }

  get inputPath(): string {
    return idToPath(this.id);
  }

  get lintedFiles(): string[] {
    return this.scannedFiles.filter((file) => !CSS_LANGS_RE.test(file));
  }

  get scannedFiles(): string[] {
    return this.files;
  }

  hasScannedFile(file: string): boolean {
    return this.fileSet.has(file);
  }

  async generate(content: string, watch: (file: string) => void): Promise<{ code: string; map: undefined } | false> {
    const inputPath = idToPath(this.id);
    const addWatchFile = (file: string) => {
      if (file === inputPath || /[#?].*\.svg$/.test(file)) return;
      watch(file);
    };

    const requiresBuild = this.requiresBuild();
    const inputBase = path.dirname(path.resolve(inputPath));

    let rebuild = false;
    if (!this.compiler || !this.scanner || (await requiresBuild)) {
      rebuild = true;
      this.buildDependencies.clear();
      await this.addBuildDependency(inputPath);
      const dependencies: Promise<void>[] = [];
      this.compiler = await compile(content, {
        base: inputBase,
        shouldRewriteUrls: true,
        onDependency: (file) => {
          addWatchFile(file);
          dependencies.push(this.addBuildDependency(file));
        },
        customCssResolver: this.cssResolver,
      });
      await Promise.all(dependencies);

      this.scanner = new Scanner({
        sources: scanSources(this.compiler, this.base),
        attributify: this.compiler.attributify,
        cache: this.cacheDir,
      });
    } else {
      for (const dependency of this.buildDependencies.keys()) addWatchFile(dependency);
    }

    const features = this.compiler.features;
    if (!(features & (Features.AtApply | Features.ThemeFunction | Features.Utilities | Features.Variants))) {
      return false;
    }

    const added: string[] = [];
    if (features & Features.Utilities) {
      for (const candidate of this.scanner.scanNew()) {
        if (this.candidates.has(candidate)) continue;
        this.candidates.add(candidate);
        added.push(candidate);
      }
      this.files = this.scanner.files;
      this.fileSet = new Set(this.files);
      for (const file of this.files) addWatchFile(file);
      for (const glob of this.scanner.globs) {
        if (glob.pattern[0] === "!") continue;
        let relative = path.relative(this.base, glob.base);
        if (relative[0] !== ".") relative = "./" + relative;
        addWatchFile(path.posix.join(normalizePath(relative), glob.pattern));
      }
    }

    return { code: this.compiler.build(rebuild ? [...this.candidates] : added), map: undefined };
  }

  private async addBuildDependency(file: string): Promise<void> {
    let mtime: number | null = null;
    try {
      mtime = (await fs.stat(file)).mtimeMs;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    this.buildDependencies.set(file, mtime);
  }

  private async requiresBuild(): Promise<boolean> {
    for (const [file, mtime] of this.buildDependencies) {
      if (mtime === null) return true;
      try {
        if ((await fs.stat(file)).mtimeMs > mtime) return true;
      } catch {
        return true;
      }
    }
    return false;
  }
}

function isScannedFile(file: string, modules: EnvironmentModuleNode[], roots: Map<string, Root>): boolean {
  const seen = new Set<EnvironmentModuleNode>();
  const queue = [...modules];
  let next = 0;
  let realpath: string | null | undefined;
  const resolveRealpath = () => {
    if (realpath === undefined) {
      try {
        realpath = realpathSync(file);
      } catch {
        realpath = null;
      }
    }
    return realpath;
  };

  while (next < queue.length) {
    const mod = queue[next++]!;
    if (seen.has(mod)) continue;
    seen.add(mod);
    const root = mod.id ? roots.get(mod.id) : undefined;
    if (root) {
      if (root.hasScannedFile(file)) return true;
      const real = resolveRealpath();
      if (real && root.hasScannedFile(real)) return true;
    }
    for (const importer of mod.importers) queue.push(importer);
  }
  return false;
}
