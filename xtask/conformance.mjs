import { existsSync, mkdirSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { join } from "node:path";

const [dir, tailwind] = process.argv.slice(2);
const require = createRequire(join(tailwind, "noop.js"));
const { compile, __unstable__loadDesignSystem } = require("@tailwindcss/node");
const tailwindDir = join(tailwind, "tailwindcss");
const COLLAPSED = 32;
const EXTRA = ["canon", "classes"];
const builtin = { tailwindcss: "index.css" };
const linked = join(dir, "node_modules", "tailwindcss");
if (!existsSync(linked)) {
  mkdirSync(join(dir, "node_modules"), { recursive: true });
  symlinkSync(tailwindDir, linked);
}

const customCssResolver = async (id) => {
  if (id in builtin) return join(tailwindDir, builtin[id]);
  if (id.startsWith("tailwindcss/")) return join(tailwindDir, id.slice(12).replace(/(\.css)?$/, ".css"));
  return undefined;
};

const own = (name) => [`${name}.css`, `${name}.txt`, `${name}.canon`, `${name}.classes`].filter((file) => existsSync(join(dir, file)));
const fixture = /\.(css|txt|canon|classes)$/;
const shared = readdirSync(dir, { withFileTypes: true })
  .filter((entry) => entry.isFile())
  .filter(({ name }) => !name.startsWith(".") && !fixture.test(name) && !name.includes(".out") && !name.endsWith(".key"))
  .map((entry) => entry.name)
  .sort();
const inputs = [
  new URL(import.meta.url),
  join(tailwindDir, "package.json"),
  join(tailwind, "@tailwindcss/node/package.json"),
  ...shared.map((file) => join(dir, file)),
];
const key = (name) => {
  const hash = createHash("sha256");
  for (const file of [...inputs, ...own(name).map((file) => join(dir, file))]) hash.update(String(file)).update(readFileSync(file));
  return hash.digest("hex");
};
const cached = (name, digest) => {
  const extra = EXTRA.filter((ext) => existsSync(join(dir, `${name}.${ext}`)));
  const outputs = [`${name}.tailwind.out`, ...extra.map((ext) => `${name}.${ext}.tailwind.out`)];
  const stored = join(dir, `${name}.tailwind.key`);
  return outputs.every((file) => existsSync(join(dir, file))) && existsSync(stored) && readFileSync(stored, "utf8") === digest;
};

const run = async (name) => {
  const digest = key(name);
  if (cached(name, digest)) return;
  await generate(name);
  writeFileSync(join(dir, `${name}.tailwind.key`), digest);
};

const generate = async (name) => {
  const css = readFileSync(join(dir, `${name}.css`), "utf8");
  const candidates = readFileSync(join(dir, `${name}.txt`), "utf8").split(/\s+/).filter(Boolean);
  try {
    const compiler = await compile(css, { base: dir, onDependency: () => {}, customCssResolver });
    writeFileSync(join(dir, `${name}.tailwind.out`), compiler.build(candidates));
  } catch (error) {
    writeFileSync(join(dir, `${name}.tailwind.out`), `error: ${error.message}`);
  }
  const canon = join(dir, `${name}.canon`);
  if (existsSync(canon)) {
    const list = readFileSync(canon, "utf8").split(/\s+/).filter(Boolean);
    const canonicalize = async (candidates, options) => {
      const ds = await __unstable__loadDesignSystem(css, { base: dir });
      return ds.canonicalizeCandidates(candidates, options).join(" ") + "\n";
    };
    let out = "";
    for (const options of [{}, { rem: 16 }]) {
      for (const candidate of list) out += await canonicalize([candidate], options);
    }
    out += await canonicalize(list.slice(0, COLLAPSED), { rem: 16, collapse: true });
    writeFileSync(join(dir, `${name}.canon.tailwind.out`), out);
  }
  if (existsSync(join(dir, `${name}.classes`))) {
    const ds = await __unstable__loadDesignSystem(css, { base: dir });
    let out = "";
    for (const [className, { modifiers }] of ds.getClassList()) out += [className, ...modifiers].join(" ") + "\n";
    for (const { name, isArbitrary, hasDash, values } of ds.getVariants()) {
      out += [`@${name}`, isArbitrary, hasDash, ...values].join(" ") + "\n";
    }
    writeFileSync(join(dir, `${name}.classes.tailwind.out`), out);
  }
};

const names = readdirSync(dir)
  .filter((file) => file.endsWith(".css") && !file.includes(".out"))
  .map((file) => file.slice(0, -4));
for (const name of names) {
  await run(name);
}
