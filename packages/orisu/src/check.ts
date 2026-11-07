import fs from "node:fs/promises";
import path from "node:path";
import { formatOptions } from "./args.ts";
import { parseArgs } from "node:util";
import { check as lint, compile, Scanner, scanSources, type Diagnostic } from "./index.ts";

const OPTIONS = {
  input: { type: "string", short: "i", description: "CSS entry file that defines the design system" },
  fix: { type: "boolean", description: "Apply every available fix in place" },
  json: { type: "boolean", description: "Print diagnostics as JSON lines" },
  help: { type: "boolean", short: "h", description: "Display usage information" },
} as const;

const help = () => {
  const out = ["Usage:", "  orisu check --input app.css [--fix] [--json] [files...]", "", "Options:"];
  out.push(...formatOptions(OPTIONS));
  out.push("", "Without files, checks every source file the entry scans.");
  process.stderr.write(out.join("\n") + "\n");
};

const scannedFiles = async (entry: string): Promise<string[]> => {
  const base = path.dirname(entry);
  const compiler = await compile(await fs.readFile(entry, "utf8"), { base });
  const sources = scanSources(compiler, base);
  const scanner = new Scanner({ sources, attributify: compiler.attributify });
  return scanner.files.filter((file) => !file.endsWith(".css"));
};

export const check = async (argv: string[]): Promise<number> => {
  const { values, positionals } = parseArgs({ args: argv, options: OPTIONS, allowPositionals: true, strict: true });
  if (values.help) {
    help();
    return 0;
  }
  if (!values.input) {
    help();
    return 1;
  }
  const entry = path.resolve(values.input);
  const files = positionals.length > 0 ? positionals.map((file) => path.resolve(file)) : await scannedFiles(entry);
  const diagnostics: Diagnostic[] = lint(entry, files, values.fix);
  const remaining = values.fix ? diagnostics.filter((d) => d.fixes.length === 0) : diagnostics;
  if (values.json) {
    for (const diagnostic of remaining) process.stdout.write(JSON.stringify(diagnostic) + "\n");
  } else {
    for (const diagnostic of remaining) process.stderr.write(diagnostic.rendered + "\n\n");
    const fixed = diagnostics.length - remaining.length;
    const errors = remaining.filter((d) => d.severity === "error").length;
    const summary = [`${remaining.length} problem${remaining.length === 1 ? "" : "s"} (${errors} error${errors === 1 ? "" : "s"})`];
    if (fixed > 0) summary.push(`${fixed} fixed`);
    process.stderr.write(summary.join(", ") + ` in ${files.length} file${files.length === 1 ? "" : "s"}\n`);
  }
  return remaining.some((d) => d.severity === "error") ? 1 : 0;
};
