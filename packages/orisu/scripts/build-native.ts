import { execFileSync } from "node:child_process";
import { copyFileSync, rmSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../../../", import.meta.url));
execFileSync("cargo", ["build", "--release", "-p", "orisu_napi"], { cwd: root, stdio: "inherit" });

const library = {
  linux: "liborisu_napi.so",
  darwin: "liborisu_napi.dylib",
  win32: "orisu_napi.dll",
}[process.platform as string];
if (!library) throw new Error(`unsupported platform ${process.platform}`);

const addon = fileURLToPath(new URL(`../orisu.${process.platform}-${process.arch}.node`, import.meta.url));
rmSync(addon, { force: true });
copyFileSync(`${root}target/release/${library}`, addon);
