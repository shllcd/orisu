#!/usr/bin/env node
// Portions ported from @tailwindcss/cli (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

import { existsSync, watch, type FSWatcher } from "node:fs";
import fs from "node:fs/promises";
import path from "node:path";
import { parseArgs, stripVTControlCharacters } from "node:util";
import { canonicalize } from "./canonicalize.ts";
import { check } from "./check.ts";
import { manifest } from "./manifest.ts";
import { compile, lsp, optimize, Scanner, type Compiler } from "./index.ts";
import type { ChangedContent, SourceEntry } from "./native.ts";

const VERSION = "0.1.0";
const DEFAULT_INPUT = '@import "tailwindcss";\n';
const DEFAULT_POLL_INTERVAL_MS = 250;
const DEBOUNCE_MS = 10;

const OPTIONS = {
  input: { type: "string", short: "i", description: "Input file" },
  output: { type: "string", short: "o", default: "-", description: "Output file" },
  watch: {
    type: "string",
    short: "w",
    description: "Watch for changes and rebuild as needed, and use `always` to keep watching when stdin is closed",
  },
  poll: { type: "string", description: "Use polling instead of filesystem events when watching" },
  minify: { type: "boolean", short: "m", description: "Optimize and minify the output" },
  optimize: { type: "boolean", description: "Optimize the output without minifying" },
  "merge-table": { type: "string", description: "Write a module that registers the class conflicts of the output for `orisu/cn`" },
  cwd: { type: "string", default: ".", description: "The current working directory" },
  silent: { type: "boolean", description: "Suppress non-error output" },
  help: { type: "boolean", short: "h", description: "Display usage information" },
} as const;

const dim = (s: string) => `\x1B[2m${s}\x1B[22m`;
const red = (s: string) => `\x1B[31m${s}\x1B[39m`;
const blue = (s: string) => `\x1B[34m${s}\x1B[39m`;
const green = (s: string) => `\x1B[32m${s}\x1B[39m`;
const yellow = (s: string) => `\x1B[33m${s}\x1B[39m`;
const eprintln = (value = "") => process.stderr.write(`${value}\n`);
const highlight = (file: string) => `${dim(blue("`"))}${blue(file)}${dim(blue("`"))}`;

const relative = (to: string) => {
  let result = path.relative(process.cwd(), to);
  if (!result.startsWith("..")) result = `.${path.sep}${result}`;
  return result.length > to.length ? to : result;
};

const formatDuration = (ns: bigint) => {
  let value = ns;
  let formatted = `${value}ns`;
  for (const unit of ["µs", "ms", "s"]) {
    if (value < 1_000n) break;
    value /= 1_000n;
    formatted = `${value}${unit}`;
  }
  if (ns <= 50_000_000n) return green(formatted);
  if (ns <= 300_000_000n) return blue(formatted);
  if (ns <= 1_000_000_000n) return yellow(formatted);
  return red(formatted);
};

const wordWrap = (text: string, width: number) => {
  const lines: string[] = [];
  let line = "";
  let length = 0;
  for (const word of text.split(" ")) {
    const wordLength = stripVTControlCharacters(word).length;
    if (length > 0 && length + wordLength + 1 > width) {
      lines.push(line);
      line = "";
      length = 0;
    }
    line += (length ? " " : "") + word;
    length += wordLength + (length ? 1 : 0);
  }
  if (length) lines.push(line);
  return lines;
};

const formatError = (error: unknown): string => {
  const seen = new Set<unknown>();
  const render = (err: unknown, depth: number): string[] => {
    const indent = "  ".repeat(depth);
    const width = (process.stderr.columns ?? Infinity) - 2 * (depth + 1);
    const output = [`${indent}${red(depth === 0 ? "Error:" : "Caused by:")}`, `${indent}${dim("┌")}`];
    for (const line of `${err}`.split("\n")) {
      const wrapped = wordWrap(line, width);
      for (const chunk of wrapped.length ? wrapped : [""]) output.push(`${indent}${dim("│")} ${chunk}`);
    }
    output.push(`${indent}${dim("└")}`);
    if (typeof err === "object" && err !== null && "cause" in err && err.cause != null && !seen.add(err).has(err.cause)) {
      output.push(...render(err.cause, depth + 1));
    }
    return output;
  };
  return render(error, 0).join("\n");
};

const fail = (message: string): never => {
  eprintln(message);
  process.exit(1);
};

const drainStdin = () =>
  new Promise<string>((resolve, reject) => {
    let result = "";
    process.stdin.on("data", (chunk) => (result += chunk));
    process.stdin.on("end", () => resolve(result));
    process.stdin.on("error", reject);
  });

const outputFile = async (file: string, contents: string) => {
  const special = await fs
    .stat(file)
    .then((stats) => stats.isCharacterDevice() || stats.isFIFO())
    .catch(() => false);
  if (!special) {
    try {
      if ((await fs.readFile(file, "utf8")) === contents) return;
    } catch {}
  }
  await fs.mkdir(path.dirname(file), { recursive: true });
  await fs.writeFile(file, contents, "utf8");
};

const cacheDir = (base: string) => {
  const modules = path.join(base, "node_modules");
  return existsSync(modules) ? path.join(modules, ".cache", "orisu") : undefined;
};

const help = (invalid?: string) => {
  const out: string[] = [];
  if (invalid) out.push(`${red("Invalid command:")} ${invalid}`, "");
  out.push(`${blue("≈")} orisu ${blue(`v${VERSION}`)}`, "", "Usage:");
  out.push(
    `  orisu [build] [--input input.css] [--output output.css] [--watch] [--poll=ms] [options…]`,
    "  orisu check --input input.css [--fix] [--json] [files...]",
    "  orisu canonicalize [classes...]",
    "  orisu manifest [dir] [-o orisu.json]",
    "  orisu lsp",
  );
  out.push(
    "",
    "Commands:",
    `  build ${dim("·".repeat(10))} Build your CSS`,
    `  check ${dim("·".repeat(10))} Report and fix class diagnostics`,
    `  canonicalize ${dim("·".repeat(3))} Canonicalize candidate lists`,
    `  manifest ${dim("·".repeat(7))} Write the candidate manifest of a library`,
    `  lsp ${dim("·".repeat(12))} Start the language server on stdio`,
  );
  out.push("", "Options:");
  for (const [name, option] of Object.entries(OPTIONS)) {
    const flag = ("short" in option ? `-${option.short}, ` : "    ") + `--${name}`;
    const fallback = "default" in option ? ` ${dim(`[default: ${option.default}]`)}` : "";
    out.push(`  ${flag.padEnd(18)}${dim("·".repeat(4))} ${option.description}${fallback}`);
  }
  eprintln(out.join("\n"));
};

interface Args {
  input?: string;
  output: string;
  watch?: string;
  poll: number | false;
  minify?: boolean;
  optimize?: boolean;
  "merge-table"?: string;
  cwd: string;
  silent?: boolean;
}

const parse = (argv: string[]): Args => {
  const normalized = argv.map((arg) => (arg === "--watch" || arg === "-w" ? "--watch=true" : arg === "--poll" ? "--poll=true" : arg));
  const { values } = parseArgs({ args: normalized, options: OPTIONS, allowPositionals: false, strict: true });
  if (values.help) {
    help();
    process.exit(0);
  }
  const poll = values.poll === undefined ? false : values.poll === "true" ? DEFAULT_POLL_INTERVAL_MS : Number(values.poll);
  if (poll !== false && !(poll > 0)) fail("Specified polling interval must be a positive number.");
  return { ...values, output: values.output, cwd: values.cwd, poll } as Args;
};

const run = async (args: Args) => {
  if (!args.silent) eprintln(`${blue("≈")} orisu ${blue(`v${VERSION}`)}\n`);

  const base = path.resolve(args.cwd);
  if (args.output !== "-") args.output = path.resolve(base, args.output);
  const mergeTableFile = args["merge-table"] && path.resolve(base, args["merge-table"]);
  if (args.input && args.input !== "-") {
    args.input = path.resolve(base, args.input);
    if (!existsSync(args.input)) fail(`Specified input file ${highlight(relative(args.input))} does not exist.`);
  }
  if (args.input === args.output && args.input !== "-") {
    fail(`Specified input file ${highlight(relative(args.input!))} and output file ${highlight(relative(args.output))} are identical.`);
  }

  const readInput = async () =>
    args.input ? (args.input === "-" ? await drainStdin() : await fs.readFile(args.input, "utf8")) : DEFAULT_INPUT;
  const inputFile = args.input && args.input !== "-" ? args.input : null;
  const inputBase = inputFile ? path.dirname(inputFile) : process.cwd();

  const previous = { css: "", optimized: "", output: null as string | null };
  const write = async (css: string) => {
    let output = css;
    if (args.minify || args.optimize) {
      if (css !== previous.css) {
        previous.css = css;
        previous.optimized = await optimize(css, { file: args.input ?? "input.css", minify: args.minify ?? false });
      }
      output = previous.optimized;
    }
    if (args.output !== "-") await outputFile(args.output, output);
    else if (output !== previous.output) process.stdout.write(`${output}\n`);
    previous.output = output;
    if (mergeTableFile) {
      const table = JSON.stringify(compiler.mergeTable([...seen]));
      await outputFile(mergeTableFile, `import { registerMergeTable } from "orisu/cn";\nregisterMergeTable(${table});\n`);
    }
  };

  const seen = new Set<string>();
  const build = (candidates: string[]) => {
    for (const candidate of candidates) seen.add(candidate);
    return compiler.build(candidates);
  };

  let fullRebuildPaths: string[] = inputFile ? [inputFile] : [];
  let backupRebuildPaths = fullRebuildPaths;

  const createCompiler = async (css: string): Promise<[Compiler, InstanceType<typeof Scanner>]> => {
    const compiler = await compile(css, { base: inputBase, onDependency: (file) => fullRebuildPaths.push(file) });
    seen.clear();
    const root = compiler.root;
    const sources: SourceEntry[] =
      root === "none" ? [] : root === null ? [{ base, pattern: "**/*", negated: false }] : [{ ...root, negated: false }];
    sources.push(...compiler.sources);
    sources.push({ base: path.dirname(process.execPath), pattern: path.basename(process.execPath), negated: true });
    if (inputFile) sources.push({ base: path.dirname(inputFile), pattern: path.basename(inputFile), negated: false });
    return [compiler, new Scanner({ sources, attributify: compiler.attributify, cache: cacheDir(base) })];
  };

  const done = (start: bigint) => {
    if (!args.silent) eprintln(`Done in ${formatDuration(process.hrtime.bigint() - start)}`);
  };

  const start = process.hrtime.bigint();
  let compiler: Compiler;
  let scanner: InstanceType<typeof Scanner>;
  try {
    [compiler, scanner] = await createCompiler(await readInput());
    await write(build(scanner.scan()));
  } catch (error) {
    return fail(formatError(error));
  }
  done(start);

  if (args.watch === undefined) return;

  const fullRebuild = async () => {
    backupRebuildPaths = fullRebuildPaths.slice();
    fullRebuildPaths = inputFile ? [inputFile] : [];
    [compiler, scanner] = await createCompiler(await readInput());
    backupRebuildPaths = fullRebuildPaths.slice();
    return build(scanner.scan());
  };

  const rebuild = async (files: string[]) => {
    const start = process.hrtime.bigint();
    try {
      if (files.some((file) => fullRebuildPaths.includes(file))) {
        await write(await fullRebuild());
        restartWatchers();
      } else {
        const changed: ChangedContent[] = files.map((file) => ({ file, extension: path.extname(file).slice(1) }));
        const candidates = scanner.scanFiles(changed);
        if (candidates.length > 0) await write(build(candidates));
      }
    } catch (error) {
      fullRebuildPaths = backupRebuildPaths;
      eprintln(formatError(error));
    }
    done(start);
  };

  let watchers: FSWatcher[] = [];
  let timer: ReturnType<typeof setTimeout> | null = null;
  const stop = () => {
    for (const watcher of watchers) watcher.close();
    watchers = [];
    if (timer) clearTimeout(timer);
  };

  const restartWatchers = () => {
    if (args.poll !== false) return;
    for (const watcher of watchers) watcher.close();
    const pending = new Set<string>();
    let queued: ReturnType<typeof setTimeout> | null = null;
    const directories = new Set<string>();
    for (const entry of scanner.normalizedSources) directories.add(path.resolve(entry.base));
    for (const file of fullRebuildPaths) directories.add(path.dirname(file));
    const sorted = [...directories].sort((a, b) => a.length - b.length);
    const roots = sorted.filter((dir, i) => !sorted.slice(0, i).some((parent) => dir.startsWith(parent + path.sep)));
    watchers = roots.flatMap((dir) => {
      try {
        return [
          watch(dir, { recursive: true }, (_, name) => {
            if (!name) return;
            const file = path.join(dir, name.toString());
            if (file === args.output) return;
            pending.add(file);
            if (queued) clearTimeout(queued);
            queued = setTimeout(() => {
              const files = [...pending];
              pending.clear();
              void rebuild(files);
            }, DEBOUNCE_MS);
          }),
        ];
      } catch {
        return [];
      }
    });
  };

  if (args.poll === false) {
    restartWatchers();
  } else {
    const interval = args.poll;
    const poll = async () => {
      const start = process.hrtime.bigint();
      try {
        const candidates = scanner.scan();
        const files = scanner.scannedFiles.filter((file) => file !== args.output);
        if (files.length > 0) {
          if (files.some((file) => fullRebuildPaths.includes(file))) {
            await write(await fullRebuild());
            done(start);
          } else if (candidates.length > 0) {
            await write(build(candidates));
            done(start);
          }
        }
      } catch (error) {
        fullRebuildPaths = backupRebuildPaths;
        eprintln(formatError(error));
      }
      timer = setTimeout(poll, interval);
    };
    timer = setTimeout(poll, interval);
  }

  if (args.watch !== "always") process.stdin.on("end", () => (stop(), process.exit(0)));
  process.stdin.resume();
};

const argv = process.argv.slice(2);
const command = argv[0];
if (command === "build") {
  await run(parse(argv.slice(1)));
} else if (command === "canonicalize") {
  process.exitCode = await canonicalize(argv.slice(1));
} else if (command === "check") {
  process.exitCode = await check(argv.slice(1));
} else if (command === "manifest") {
  process.exitCode = await manifest(argv.slice(1));
} else if (command === "lsp") {
  lsp();
} else if (command && !command.startsWith("-")) {
  help(command);
  process.exit(1);
} else if (process.stdout.isTTY && command === undefined) {
  help();
} else {
  await run(parse(argv));
}
