import { cpSync } from "node:fs";
import { fileURLToPath } from "node:url";

cpSync(fileURLToPath(new URL("../../../presets/", import.meta.url)), fileURLToPath(new URL("../presets/", import.meta.url)), {
  recursive: true,
  filter: (source) => !source.endsWith(".md"),
});
