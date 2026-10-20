import { readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";

const [work, command, scenario, k, mode] = process.argv.slice(2);
const { entry, root, modules, orisu } = JSON.parse(readFileSync(join(work, "config.json"), "utf8"));
const require = createRequire(join(modules, "noop.js"));
const base = dirname(entry);
const css = readFileSync(entry, "utf8");
const twDir = dirname(require.resolve("tailwindcss/package.json"));
const twPath = (id) => (id === "tailwindcss" ? join(twDir, "index.css") : join(twDir, id.slice(12).replace(/(\.css)?$/, ".css")));
const customCssResolver = async (id) => (id === "tailwindcss" || id.startsWith("tailwindcss/") ? twPath(id) : undefined);
const absoluteCss = css.replace(/@import\s+(["'])(tailwindcss(?:\/[^"']*)?)\1/g, (_, quote, id) => `@import ${quote}${twPath(id)}${quote}`);

const sourcesOf = (compiler) => {
  const top = compiler.root;
  const sources = top === "none" ? [] : top === null ? [{ base: root, pattern: "**/*", negated: false }] : [{ ...top, negated: false }];
  return sources.concat(compiler.sources);
};

const engines = {
  tailwind: async () => {
    const { compile, __unstable__loadDesignSystem } = require("@tailwindcss/node");
    const { Scanner } = require("@tailwindcss/oxide");
    return {
      compile: () => compile(css, { base, onDependency: () => {}, customCssResolver }),
      scanner: (compiler, extra = {}) => new Scanner({ sources: sourcesOf(compiler), ...extra }),
      loadDs: () => __unstable__loadDesignSystem(absoluteCss, { base }),
    };
  },
  orisu: async () => {
    const { compile, Scanner } = await import(join(orisu, "index.js"));
    return {
      compile: () => compile(css, { base }),
      scanner: (compiler, extra = {}) => new Scanner({ sources: sourcesOf(compiler), attributify: compiler.attributify, ...extra }),
      loadDs: () => compile(css, { base }),
    };
  },
};

const cnCalls = () => {
  const calls = [];
  const walk = (dir) => {
    for (const item of readdirSync(dir, { withFileTypes: true })) {
      if (item.name === "node_modules" || item.name.startsWith(".")) continue;
      const path = join(dir, item.name);
      if (item.isDirectory()) walk(path);
      else if (/\.tsx?$/.test(item.name)) {
        for (const call of readFileSync(path, "utf8").matchAll(/\bcn\(([^()]*(?:\([^()]*\)[^()]*)*)\)/g)) {
          const args = [...call[1].matchAll(/"([^"]*)"/g)].map((arg) => arg[1]).filter(Boolean);
          if (args.length > 0) calls.push(args.join(" "));
        }
      }
    }
  };
  walk(root);
  return calls;
};

const prepare = async () => {
  const out = {};
  for (const name of ["tailwind", "orisu"]) {
    const engine = await engines[name]();
    const compiler = await engine.compile();
    const scanner = engine.scanner(compiler);
    const candidates = scanner.scan();
    out[name] = { css: compiler.build(candidates), candidates: candidates.sort(), files: scanner.files };
  }
  const { tailwind, orisu } = out;
  if (JSON.stringify(tailwind.candidates) !== JSON.stringify(orisu.candidates)) throw new Error("scanned candidates differ");
  if (tailwind.css !== orisu.css) {
    writeFileSync(join(work, "tailwind.css"), tailwind.css);
    writeFileSync(join(work, "orisu.css"), orisu.css);
    throw new Error(`CSS differs: ${join(work, "tailwind.css")} vs ${join(work, "orisu.css")}`);
  }
  const hmrFile = orisu.files.find((file) => file.endsWith(".tsx") && readFileSync(file, "utf8").includes("className=\""));
  if (!hmrFile) throw new Error("no .tsx file with className=\" to edit");
  const calls = cnCalls();
  writeFileSync(join(work, "data.json"), JSON.stringify({ candidates: orisu.candidates, cn: calls, hmrFile }));
  process.stdout.write(
    `${orisu.files.length} files, ${orisu.candidates.length} candidates, ${orisu.css.length} bytes of identical CSS, ` +
      `${calls.length} cn calls (${new Set(calls).size} distinct)\n`,
  );
};

const run = async (impl) => {
  const K = Number(k);
  const { candidates, cn: calls, hmrFile } = JSON.parse(readFileSync(join(work, "data.json"), "utf8"));
  const hmrSource = readFileSync(hmrFile, "utf8");
  const engine = await engines[impl]();
  let seed = 7;
  const pick = () => ((seed = (seed * 1103515245 + 12345) & 0x7fffffff), candidates[seed % candidates.length]);
  const cnOf = async () => {
    if (impl === "tailwind") {
      const { twMerge } = require("tailwind-merge");
      const { clsx } = require("clsx");
      return (...args) => twMerge(clsx(args));
    }
    const { cn, registerMergeTable } = await import(join(orisu, "cn.js"));
    registerMergeTable((await engine.compile()).mergeTable(candidates));
    return cn;
  };

  const scenarios = {
    compile: async () => async () => {
      for (let i = 0; i < K; i++) await engine.compile();
    },
    scan: async () => {
      const compiler = await engine.compile();
      engine.scanner(compiler).scan();
      return () => {
        for (let i = 0; i < K; i++) engine.scanner(compiler).scan();
      };
    },
    "scan-cached": async () => {
      const compiler = await engine.compile();
      const cache = join(work, "scanner-cache");
      rmSync(cache, { recursive: true, force: true });
      engine.scanner(compiler, { cache }).scan();
      return () => {
        for (let i = 0; i < K; i++) engine.scanner(compiler, { cache }).scan();
      };
    },
    build: async () => {
      const compilers = [];
      for (let i = 0; i < K; i++) compilers.push(await engine.compile());
      return () => {
        for (const compiler of compilers) compiler.build(candidates);
      };
    },
    "cold-start": async () => async () => {
      for (let i = 0; i < K; i++) {
        const compiler = await engine.compile();
        compiler.build(engine.scanner(compiler).scan());
      }
    },
    hmr: async () => {
      const compiler = await engine.compile();
      const scanner = engine.scanner(compiler);
      compiler.build(scanner.scan());
      const edits = Array.from({ length: K }, (_, i) => hmrSource.replace("className=\"", `className="w-[${i + 1}.25px] `));
      return () => {
        for (const content of edits) compiler.build(scanner.scanFiles([{ content, extension: "tsx" }]));
      };
    },
    "hmr-no-new-class": async () => {
      const compiler = await engine.compile();
      const scanner = engine.scanner(compiler);
      compiler.build(scanner.scan());
      return () => {
        for (let i = 0; i < K; i++) compiler.build(scanner.scanFiles([{ content: `${hmrSource}\n// ${i}`, extension: "tsx" }]));
      };
    },
    "canonicalize-cold": async () => {
      const systems = [];
      for (let i = 0; i < K; i++) systems.push(await engine.loadDs());
      const list = Array.from({ length: 200 }, pick);
      return () => {
        for (const ds of systems) ds.canonicalizeCandidates(list, { rem: 16 });
      };
    },
    "canonicalize-warm": async () => {
      const ds = await engine.loadDs();
      ds.canonicalizeCandidates(["p-4"], { rem: 16 });
      const lists = Array.from({ length: K }, () => Array.from({ length: 50 }, pick));
      return () => {
        for (const list of lists) ds.canonicalizeCandidates(list, { rem: 16 });
      };
    },
    cn: async () => {
      const cn = await cnOf();
      cn("p-2 p-4");
      const batch = Array.from({ length: K }, (_, i) => calls[i % calls.length]);
      return () => {
        for (const call of batch) cn(call);
      };
    },
    "cn-once": async () => {
      const cn = await cnOf();
      cn("p-2 p-4");
      const distinct = [...new Set(calls)];
      return () => {
        for (const call of distinct) cn(call);
      };
    },
  };

  const measured = await scenarios[scenario]();
  if (mode === "1") {
    const start = process.hrtime.bigint();
    await measured();
    process.stdout.write(`${Number(process.hrtime.bigint() - start) / 1e6}\n`);
  }
};

await (command === "prepare" ? prepare() : run(command));
