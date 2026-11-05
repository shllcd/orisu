import { spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [work, engine, scenario, k, mode] = process.argv.slice(2);
const { root, file, servers, counter } = JSON.parse(readFileSync(join(work, "lsp.json"), "utf8"));
const K = Number(k);
const uri = pathToFileURL(file).href;
const source = readFileSync(file, "utf8");

const at = (offset) => {
  const lines = source.slice(0, offset).split("\n");
  return { line: lines.length - 1, character: lines.at(-1).length };
};
const attribute = source.indexOf("className=\"");
const firstClass = source.indexOf(" ", attribute) + 1;
const completionAt = at(firstClass);
const hoverAt = at(attribute + "className=\"".length + 1);

const settings = { tailwindCSS: {} };
const capabilities = {
  general: { positionEncodings: ["utf-16"] },
  textDocument: {
    synchronization: { didSave: true },
    completion: {
      completionItem: { snippetSupport: true, documentationFormat: ["markdown", "plaintext"], resolveSupport: { properties: ["documentation", "detail"] } },
      completionList: { itemDefaults: ["commitCharacters", "editRange", "insertTextFormat", "insertTextMode", "data"] },
    },
    hover: { contentFormat: ["markdown", "plaintext"] },
    publishDiagnostics: { versionSupport: true },
    colorProvider: {},
  },
  workspace: { configuration: true, workspaceFolders: true, didChangeWatchedFiles: { dynamicRegistration: true } },
};

const started = process.hrtime.bigint();
const [command, ...args] = [...counter, ...servers[engine]];
const server = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"] });
let stderr = "";
server.stderr.on("data", (chunk) => (stderr += chunk));
const exited = new Promise((resolve) => server.on("exit", resolve));

let nextId = 0;
const pending = new Map();
const listeners = new Set();
const send = (message) => {
  const body = Buffer.from(JSON.stringify({ jsonrpc: "2.0", ...message }));
  server.stdin.write(Buffer.concat([Buffer.from(`Content-Length: ${body.length}\r\n\r\n`), body]));
};
const request = (method, params) =>
  new Promise((resolve, reject) => {
    const id = ++nextId;
    pending.set(id, { resolve, reject });
    send({ id, method, params });
  });
const notify = (method, params) => send({ method, params });
const next = (method, test = () => true) =>
  new Promise((resolve) => {
    const listener = (message) => {
      if (message.method === method && test(message.params)) {
        listeners.delete(listener);
        resolve(message.params);
      }
    };
    listeners.add(listener);
  });

const onMessage = (message) => {
  if (message.id !== undefined && message.method === undefined) {
    const handler = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) handler.reject(new Error(`${engine}: ${JSON.stringify(message.error)}`));
    else handler.resolve(message.result);
    return;
  }
  if (message.id !== undefined) {
    const result = message.method === "workspace/configuration" ? message.params.items.map((item) => settings[item.section] ?? {}) : null;
    send({ id: message.id, result });
    return;
  }
  for (const listener of [...listeners]) listener(message);
};

let buffer = Buffer.alloc(0);
server.stdout.on("data", (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);
  for (;;) {
    const header = buffer.indexOf("\r\n\r\n");
    if (header < 0) return;
    const length = Number(/Content-Length: (\d+)/i.exec(buffer.subarray(0, header).toString())[1]);
    if (buffer.length < header + 4 + length) return;
    const body = buffer.subarray(header + 4, header + 4 + length).toString();
    buffer = buffer.subarray(header + 4 + length);
    onMessage(JSON.parse(body));
  }
});

const itemsOf = (result) => (Array.isArray(result) ? result : (result?.items ?? []));
const complete = () => request("textDocument/completion", { textDocument: { uri }, position: completionAt });
const hover = () => request("textDocument/hover", { textDocument: { uri }, position: hoverAt });
const colors = () => request("textDocument/documentColor", { textDocument: { uri } });
let version = 1;
const edit = async () => {
  version += 1;
  const text = source.replace("className=\"", `className="w-[${version}.25px] `);
  const published = next("textDocument/publishDiagnostics", (params) => params.uri === uri);
  notify("textDocument/didChange", { textDocument: { uri, version }, contentChanges: [{ text }] });
  return published;
};

await request("initialize", {
  processId: process.pid,
  rootUri: pathToFileURL(root).href,
  workspaceFolders: [{ uri: pathToFileURL(root).href, name: "bench" }],
  capabilities,
  initializationOptions: { testMode: true },
});
notify("initialized", {});
notify("workspace/didChangeConfiguration", { settings });
notify("textDocument/didOpen", { textDocument: { uri, languageId: "typescriptreact", version, text: source } });
let list = await complete();
while (itemsOf(list).length === 0) list = await complete();
const ready = Number(process.hrtime.bigint() - started) / 1e6;

const operations = {
  completion: complete,
  "completion-resolve": () => {
    const { itemDefaults } = list;
    const item = itemsOf(list).find((item) => item.label === "bg-red-500") ?? itemsOf(list)[0];
    return request("completionItem/resolve", { ...item, data: item.data ?? itemDefaults?.data });
  },
  hover,
  diagnostics: edit,
  colors,
};

let ms = ready;
if (scenario === "prepare") {
  const summary = `${itemsOf(await complete()).length} completions, ${(await colors()).length} colors, ${(await hover()) ? "hover" : "no hover"}`;
  await edit();
  const { diagnostics } = await edit();
  process.stdout.write(`${engine}: ${summary}, ${diagnostics.length} diagnostics: ${diagnostics.map((d) => d.code).join(" ")}\n`);
} else if (scenario !== "startup") {
  const operation = operations[scenario];
  for (let i = 0; i < 3; i++) await operation();
  await new Promise((resolve) => setTimeout(resolve, 200));
  const start = process.hrtime.bigint();
  if (mode === "1") for (let i = 0; i < K; i++) await operation();
  ms = Number(process.hrtime.bigint() - start) / 1e6;
}

await request("shutdown", null);
notify("exit", null);
await exited;
process.stderr.write(stderr);
if (scenario !== "prepare") process.stdout.write(`${ms}\n`);
