// Portions ported from @tailwindcss/cli (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

import fs from "node:fs/promises";
import path from "node:path";
import { createInterface } from "node:readline";
import { parseArgs } from "node:util";
import { compile, type Compiler } from "./index.ts";

type OutputFormat = "text" | "json" | "jsonl";

const CLOSING: Record<number, number> = { 0x28: 0x29, 0x5b: 0x5d, 0x7b: 0x7d };

/** Splits on a top-level `separator`, skipping brackets, quotes and escapes. */
const segment = (input: string, separator: string): string[] => {
  const stack: number[] = [];
  const parts: string[] = [];
  const code = separator.charCodeAt(0);
  let last = 0;
  for (let i = 0; i < input.length; i++) {
    const c = input.charCodeAt(i);
    if (stack.length === 0 && c === code) {
      parts.push(input.slice(last, i));
      last = i + 1;
      continue;
    }
    if (c === 0x5c) {
      i++;
    } else if (c === 0x22 || c === 0x27) {
      while (++i < input.length) {
        const next = input.charCodeAt(i);
        if (next === 0x5c) i++;
        else if (next === c) break;
      }
    } else if (CLOSING[c] !== undefined) {
      stack.push(CLOSING[c]!);
    } else if ((c === 0x29 || c === 0x5d || c === 0x7d) && stack[stack.length - 1] === c) {
      stack.pop();
    }
  }
  parts.push(input.slice(last));
  return parts;
};

interface CandidateGroupResult {
  input: string;
  output: string;
  changed: boolean;
}

const OPTIONS = {
  css: { type: "string", description: 'CSS entry file used to load the design system (defaults to `@import "tailwindcss";`)' },
  format: { type: "string", default: "text", description: "Output format: text, json or jsonl" },
  stream: { type: "boolean", default: false, description: "Read candidate groups from stdin line by line and write results to stdout" },
  help: { type: "boolean", short: "h", default: false, description: "Display usage information" },
} as const;

const helpMessage = () => {
  const out = [
    "Usage:",
    "  orisu canonicalize [classes...]",
    "  orisu canonicalize --css input.css [classes...]",
    "  orisu canonicalize --stream [--css input.css]",
  ];
  out.push("", "Options:");
  for (const [name, option] of Object.entries(OPTIONS)) {
    const flag = ("short" in option ? `-${option.short}, ` : "    ") + `--${name}`;
    const fallback = "default" in option ? ` [default: ${option.default}]` : "";
    out.push(`  ${flag.padEnd(16)}···· ${option.description}${fallback}`);
  }
  return out.join("\n");
};

const parseFormat = (input: string): OutputFormat => {
  if (input === "text" || input === "json" || input === "jsonl") return input;
  throw new Error(`Invalid value for --format: ${input}`);
};

const load = async (cssFile: string | undefined, cwd: string) => {
  if (cssFile === undefined) return compile('@import "tailwindcss";\n', { base: cwd });
  const file = path.resolve(cwd, cssFile);
  return compile(await fs.readFile(file, "utf8"), { base: path.dirname(file) });
};

const ZERO = 48;
const NINE = 57;
const isDigit = (code: number) => code >= ZERO && code <= NINE;

const compare = (a: string, z: string) => {
  const minLen = Math.min(a.length, z.length);
  for (let i = 0; i < minLen; i++) {
    const aCode = a.charCodeAt(i);
    const zCode = z.charCodeAt(i);
    if (isDigit(aCode) && isDigit(zCode)) {
      let aEnd = i + 1;
      while (isDigit(a.charCodeAt(aEnd))) aEnd++;
      let zEnd = i + 1;
      while (isDigit(z.charCodeAt(zEnd))) zEnd++;
      const aNumber = a.slice(i, aEnd);
      const zNumber = z.slice(i, zEnd);
      const diff = Number(aNumber) - Number(zNumber);
      if (diff) return diff;
      if (aNumber < zNumber) return -1;
      if (aNumber > zNumber) return 1;
      continue;
    }
    if (aCode !== zCode) return aCode - zCode;
  }
  return a.length - z.length;
};

const defaultSort = (candidates: string[], order: (number | null)[]) =>
  candidates
    .map((candidate, i) => [candidate, order[i]] as const)
    .sort(([candidateA, orderA], [candidateZ, orderZ]) => {
      if (orderA === orderZ) return compare(candidateA, candidateZ);
      if (orderA === null) return -1;
      if (orderZ === null) return 1;
      return Math.sign(orderA - orderZ) || compare(candidateA, candidateZ);
    })
    .map(([candidate]) => candidate);

const splitCandidates = (input: string) => {
  const trimmed = input.trim();
  if (trimmed.length === 0) return [];
  return segment(trimmed, " ")
    .map((candidate) => candidate.trim())
    .filter((candidate) => candidate.length > 0);
};

const canonicalizeGroup = (compiler: Compiler, input: string): CandidateGroupResult => {
  const candidates = compiler.canonicalizeCandidates(splitCandidates(input), { collapse: true, logicalToPhysical: true });
  const output = defaultSort(candidates, compiler.getClassOrder(candidates)).join(" ");
  return { input, output, changed: output !== input };
};

const splitCandidateGroups = (input: string) =>
  input
    .split(/\r?\n/g)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);

const drainStdin = () =>
  new Promise<string>((resolve, reject) => {
    let result = "";
    process.stdin.on("data", (chunk) => (result += chunk));
    process.stdin.on("end", () => resolve(result));
    process.stdin.on("error", reject);
  });

const indent = (input: string, size: number) =>
  input
    .split("\n")
    .map((line) => " ".repeat(size) + line)
    .join("\n");

const formatResults = (results: CandidateGroupResult[], format: OutputFormat) => {
  switch (format) {
    case "json":
      return JSON.stringify(results, null, 2);
    case "jsonl":
      return results.map((result) => JSON.stringify(result)).join("\n");
    case "text":
      return results.map((result) => result.output).join("\n");
  }
};

const stream = async (compiler: Compiler, format: OutputFormat) => {
  const output = process.stdout;
  let first = true;
  if (format === "json") output.write("[");
  for await (const line of createInterface({ input: process.stdin })) {
    const result = canonicalizeGroup(compiler, line);
    if (format === "text") output.write(result.output + "\n");
    else if (format === "jsonl") output.write(JSON.stringify(result) + "\n");
    else {
      output.write(first ? "\n" : ",\n");
      output.write(indent(JSON.stringify(result, null, 2), 2));
      first = false;
    }
  }
  if (format === "json") output.write(first ? "]" : "\n]");
};

const run = async (argv: string[]): Promise<{ exitCode: number; stdout: string; stderr: string }> => {
  try {
    const { values, positionals } = parseArgs({ args: argv, options: OPTIONS, allowPositionals: true, strict: true });
    const format = parseFormat(values.format);
    if ((process.stdout.isTTY && argv.length === 0) || values.help) return { exitCode: 0, stdout: helpMessage(), stderr: "" };
    const cwd = process.cwd();
    if (values.stream) {
      await stream(await load(values.css, cwd), format);
      return { exitCode: 0, stdout: "", stderr: "" };
    }
    const inputs = positionals.length > 0 ? positionals : process.stdin.isTTY ? [] : splitCandidateGroups(await drainStdin());
    if (inputs.length === 0) return { exitCode: 1, stdout: helpMessage(), stderr: "No candidate groups provided" };
    const compiler = await load(values.css, cwd);
    return {
      exitCode: 0,
      stdout: formatResults(
        inputs.map((input) => canonicalizeGroup(compiler, input)),
        format,
      ),
      stderr: "",
    };
  } catch (error) {
    return { exitCode: 1, stdout: "", stderr: error instanceof Error ? error.message : String(error) };
  }
};

/** The `canonicalize` command of the Tailwind CLI; resolves to the exit code. */
export const canonicalize = async (argv: string[]) => {
  const result = await run(argv);
  if (result.stdout.length > 0) process.stdout.write(`${result.stdout}\n`);
  if (result.stderr.length > 0) process.stderr.write(`${result.stderr}\n`);
  return result.exitCode;
};
