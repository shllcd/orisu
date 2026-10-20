import fs from "node:fs/promises";
import path from "node:path";
import { parseArgs } from "node:util";
import { Scanner } from "./index.ts";

const OPTIONS = {
  output: { type: "string", short: "o", default: "orisu.json", description: "Manifest file, relative to the package" },
  help: { type: "boolean", short: "h", description: "Display usage information" },
} as const;

const help = () => {
  const out = ["Usage:", "  orisu manifest [dir] [-o orisu.json]", "", "Options:"];
  for (const [name, option] of Object.entries(OPTIONS)) {
    const flag = ("short" in option ? `-${option.short}, ` : "    ") + `--${name}`;
    out.push(`  ${flag.padEnd(16)}${option.description}`);
  }
  out.push("", 'Lists every candidate of the package in `dir`. Point `"orisu": { "manifest": "./orisu.json" }` in its');
  out.push("package.json at the file so that apps read it instead of scanning the package.");
  process.stderr.write(out.join("\n") + "\n");
};

export const manifest = async (argv: string[]): Promise<number> => {
  const { values, positionals } = parseArgs({ args: argv, options: OPTIONS, allowPositionals: true, strict: true });
  if (values.help || positionals.length > 1) {
    help();
    return values.help ? 0 : 1;
  }
  const base = path.resolve(positionals[0] ?? ".");
  const output = path.resolve(base, values.output);
  const scanner = new Scanner({
    sources: [
      { base, pattern: "**/*", negated: false },
      { base: path.dirname(output), pattern: path.basename(output), negated: true },
    ],
  });
  const candidates = scanner.scan().sort();
  await fs.mkdir(path.dirname(output), { recursive: true });
  await fs.writeFile(output, JSON.stringify({ candidates }) + "\n", "utf8");
  process.stderr.write(`${candidates.length} candidates from ${scanner.files.length} files in ${path.relative(process.cwd(), output)}\n`);
  return 0;
};
