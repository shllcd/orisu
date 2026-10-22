import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../", import.meta.url));
const read = (path) => readFileSync(`${root}${path}`, "utf8");
const write = (path, content) => writeFileSync(`${root}${path}`, content);
const manifest = "packages/orisu/package.json";

const bindings = {
  "x86_64-apple-darwin": ["darwin-x64", "darwin", "x64"],
  "aarch64-apple-darwin": ["darwin-arm64", "darwin", "arm64"],
  "x86_64-pc-windows-msvc": ["win32-x64-msvc", "win32", "x64"],
  "aarch64-pc-windows-msvc": ["win32-arm64-msvc", "win32", "arm64"],
  "x86_64-unknown-linux-gnu": ["linux-x64-gnu", "linux", "x64", "glibc"],
  "aarch64-unknown-linux-gnu": ["linux-arm64-gnu", "linux", "arm64", "glibc"],
  "x86_64-unknown-linux-musl": ["linux-x64-musl", "linux", "x64", "musl"],
  "aarch64-unknown-linux-musl": ["linux-arm64-musl", "linux", "arm64", "musl"],
};

const replace = (content, pattern, replacement) => {
  if (!pattern.test(content)) throw new Error(`no match for ${pattern}`);
  return content.replace(pattern, replacement);
};

const tagExists = (version) => {
  try {
    execFileSync("git", ["rev-parse", "-q", "--verify", `refs/tags/v${version}`], { cwd: root, stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
};

const unreleased = (changelog) => {
  const match = /^## \[Unreleased\][^\n]*\n([\s\S]*?)(?=^## \[|(?![\s\S]))/m.exec(changelog);
  if (!match) throw new Error("CHANGELOG.md has no ## [Unreleased] section");
  const notes = match[1].trim();
  if (!/^\s*[-*] /m.test(notes)) throw new Error("## [Unreleased] has no entries");
  return { match, notes };
};

const bumpLevel = (notes) => {
  const headings = [...notes.matchAll(/^### (.+)$/gm)].map(([, heading]) => heading.trim().toLowerCase());
  if (headings.some((heading) => /breaking|removed/.test(heading))) return "major";
  if (headings.some((heading) => /added|changed|deprecated|features?/.test(heading))) return "minor";
  return "patch";
};

const nextVersion = (current, level) => {
  if (!tagExists(current)) return current;
  const [major, minor, patch] = current.split(/[.-]/).map(Number);
  const effective = major === 0 && level === "major" ? "minor" : level;
  if (effective === "major") return `${major + 1}.0.0`;
  if (effective === "minor") return `${major}.${minor + 1}.0`;
  return `${major}.${minor}.${patch + 1}`;
};

const bumpVersion = () => {
  const changelog = read("CHANGELOG.md");
  const { match, notes } = unreleased(changelog);
  const pkg = JSON.parse(read(manifest));
  const version = nextVersion(pkg.version, bumpLevel(notes));
  if (tagExists(version)) throw new Error(`tag v${version} already exists`);
  pkg.version = version;
  for (const name of Object.keys(pkg.optionalDependencies)) pkg.optionalDependencies[name] = version;
  write(manifest, `${JSON.stringify(pkg, null, 2)}\n`);
  write("packages/orisu/src/cli.ts", replace(read("packages/orisu/src/cli.ts"), /const VERSION = "[^"]+";/, `const VERSION = "${version}";`));
  const section = `## [Unreleased]\n\n## [${version}] - ${new Date().toISOString().slice(0, 10)}\n\n${notes}\n\n`;
  const updated = changelog.slice(0, match.index) + section + changelog.slice(match.index + match[0].length);
  write("CHANGELOG.md", `${updated.trimEnd()}\n`);
  write("notes.md", `${notes}\n`);
  console.log(version);
};

const stagePackages = () => {
  const version = JSON.parse(read(manifest)).version;
  for (const [target, [name, os, cpu, libc]] of Object.entries(bindings)) {
    const addon = `orisu.${target}.node`;
    const dir = `dist/npm/${name}`;
    mkdirSync(`${root}${dir}`, { recursive: true });
    copyFileSync(`${root}artifacts/bindings-${target}/${addon}`, `${root}${dir}/${addon}`);
    const pkg = {
      name: `@orisu/binding-${name}`,
      version,
      description: `Native binding of orisu for ${target}`,
      license: "MIT",
      os: [os],
      cpu: [cpu],
      ...(libc ? { libc: [libc] } : {}),
      main: addon,
      files: [addon],
      repository: { type: "git", url: "git+https://github.com/shllcd/orisu.git", directory: "packages/orisu" },
    };
    write(`${dir}/package.json`, `${JSON.stringify(pkg, null, 2)}\n`);
  }
};

const [, , command] = process.argv;
if (command === "version") bumpVersion();
else if (command === "packages") stagePackages();
else throw new Error("usage: release.mjs version | packages");
