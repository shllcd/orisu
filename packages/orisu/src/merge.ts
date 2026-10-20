// Caching strategy adapted from shadcn-ui/cn (https://github.com/shadcn-ui/cn),
// MIT License, Copyright (c) 2026 shadcn.

const ORDER_SENSITIVE = new Set([
  "*",
  "**",
  "before",
  "after",
  "backdrop",
  "placeholder",
  "file",
  "marker",
  "selection",
  "first-line",
  "first-letter",
  "details-content",
]);

const CACHE_SIZE = 8192;
const PARSED_SIZE = 32768;
const DOOR_SIZE = 16384;

interface ParsedClass {
  group: number;
  shape: number;
  erases: readonly number[];
}

let prefix = "";
let entries: Map<string, ParsedClass> = new Map();
let erasures: readonly (readonly number[])[] = [];
let stride = 1;

let groups = new Map<string, number>([["", 0]]);
let stamps = new Int32Array(0);
let epoch = 0;

let parsed = new Map<string, ParsedClass | null>();
let prevParsed = new Map<string, ParsedClass | null>();

let tokStart = new Int32Array(64);
let tokEnd = new Int32Array(64);
let tokPlain = new Uint8Array(64);
let keep = new Uint8Array(64);

let cache: Record<string, string> = Object.create(null);
let prevCache: Record<string, string> = Object.create(null);
let cacheCount = 0;

const door = new Int32Array(DOOR_SIZE * 2);
let doorBase = 0;
let doorEpoch = 1;
let doorMarks = 0;

/** Installs the shape table generated at build time; classes missing from it are never merged away. */
export const registerMergeTable = (table: {
  prefix?: string | null;
  keys: readonly string[];
  values: readonly number[];
  erases: readonly (readonly number[])[];
}): void => {
  prefix = table.prefix ?? "";
  erasures = table.erases;
  stride = Math.max(erasures.length, 1);
  entries = new Map();
  for (let i = 0; i < table.keys.length; i++) {
    const shape = table.values[i]!;
    entries.set(table.keys[i]!, { group: 0, shape, erases: erasures[shape]! });
  }
  groups = new Map([["", 0]]);
  stamps = new Int32Array(0);
  parsed = new Map();
  prevParsed = new Map();
  cache = Object.create(null);
  prevCache = Object.create(null);
  cacheCount = 0;
  door.fill(0);
};

/** Sampled positional hash: O(1) in the string length, so one-shot strings are cheap to screen. */
export const sampleHash = (s: string): number => {
  const len = s.length;
  if (len === 0) return 0;
  let h = Math.imul(len, 0x9e3779b1) ^ s.charCodeAt(0);
  if (len > 3) {
    const q = len >> 2;
    const m = len >> 1;
    h = Math.imul(h ^ (s.charCodeAt(1) << 8) ^ (s.charCodeAt(2) << 16) ^ s.charCodeAt(q), 0x85ebca6b);
    h = Math.imul(h ^ (s.charCodeAt(m) << 8) ^ (s.charCodeAt(m + q) << 16) ^ s.charCodeAt(len - 3), 0xc2b2ae35);
    h ^= (s.charCodeAt(len - 2) << 8) ^ (s.charCodeAt(len - 1) << 16);
    for (let i = 3, k = len - 4; i < 8 && i < k; i++, k--) {
      h = Math.imul(h ^ s.charCodeAt(i) ^ (s.charCodeAt(k) << 16), 0x27d4eb2d);
    }
  } else {
    for (let i = 1; i < len; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x85ebca6b);
  }
  return h ^ (h >>> 15);
};

const rotateDoor = () => {
  doorBase ^= DOOR_SIZE;
  doorEpoch = (doorEpoch + 1) | 0;
  doorMarks = 0;
};

/** Whether `input` was screened before; marks it otherwise. */
export const seenBefore = (input: string): boolean => {
  const hash = sampleHash(input);
  const slot = (hash & (DOOR_SIZE - 1)) + doorBase;
  if (door[slot] === (hash ^ doorEpoch) || door[slot ^ DOOR_SIZE] === (hash ^ (doorEpoch - 1))) return true;
  door[slot] = hash ^ doorEpoch;
  if (++doorMarks > DOOR_SIZE) rotateDoor();
  return false;
};

const segment = (input: string): string[] => {
  const parts: string[] = [];
  const stack: string[] = [];
  let last = 0;
  for (let i = 0; i < input.length; i++) {
    const ch = input[i];
    if (stack.length === 0 && ch === ":") {
      parts.push(input.slice(last, i));
      last = i + 1;
      continue;
    }
    switch (ch) {
      case "\\":
        i++;
        break;
      case "'":
      case '"':
        for (i++; i < input.length; i++) {
          if (input[i] === "\\") i++;
          else if (input[i] === ch) break;
        }
        break;
      case "(":
        stack.push(")");
        break;
      case "[":
        stack.push("]");
        break;
      case "{":
        stack.push("}");
        break;
      case ")":
      case "]":
      case "}":
        if (stack[stack.length - 1] === ch) stack.pop();
        break;
    }
  }
  parts.push(input.slice(last));
  return parts;
};

const groupId = (group: string): number => {
  let id = groups.get(group);
  if (id === undefined) {
    id = groups.size;
    groups.set(group, id);
  }
  return id;
};

const parseBase = (base: string, important: boolean, variants: string[] | null): ParsedClass | null => {
  if (base.endsWith("!")) {
    base = base.slice(0, -1);
    important = true;
  } else if (base.startsWith("!")) {
    base = base.slice(1);
    important = true;
  }
  const entry = entries.get(base);
  if (entry === undefined) return null;
  let group = important ? "!" : "";
  if (variants !== null) {
    if (prefix && variants[0] === prefix) variants.shift();
    let run: string[] = [];
    for (const part of variants) {
      if (part[0] === "[" || ORDER_SENSITIVE.has(part)) {
        for (const variant of run.sort()) group += variant + ":";
        run = [];
        group += part + ":";
      } else {
        run.push(part);
      }
    }
    for (const variant of run.sort()) group += variant + ":";
  }
  return group === "" ? entry : { group: groupId(group), shape: entry.shape, erases: entry.erases };
};

const parse = (raw: string): ParsedClass | null => {
  if (raw.indexOf(":") === -1) return parseBase(raw, false, null);
  const parts = segment(raw);
  const base = parts.pop()!;
  return parseBase(base, false, parts.length === 0 ? null : parts);
};

const lookup = (raw: string): ParsedClass | null => {
  let entry = parsed.get(raw);
  if (entry !== undefined) return entry;
  entry = prevParsed.get(raw);
  if (entry === undefined) entry = parse(raw);
  parsed.set(raw, entry);
  if (parsed.size >= PARSED_SIZE) {
    prevParsed = parsed;
    parsed = new Map();
  }
  return entry;
};

const isSpace = (c: number): boolean => c === 32 || (c >= 9 && c <= 13);

/** Merges without touching the result cache. */
export const mergeUncached = (input: string): string => {
  const n = input.length;
  let count = 0;
  let normalized = true;
  for (let i = 0; i < n; ) {
    let c = input.charCodeAt(i);
    if (isSpace(c)) {
      if (c !== 32 || i === 0 || isSpace(input.charCodeAt(i - 1))) normalized = false;
      i++;
      continue;
    }
    const start = i;
    let plain = 1;
    while (i < n && !isSpace((c = input.charCodeAt(i)))) {
      if (c === 58 || c === 33) plain = 0;
      i++;
    }
    if (count === tokStart.length) {
      const grown = count * 2;
      const s = new Int32Array(grown);
      s.set(tokStart);
      tokStart = s;
      const e = new Int32Array(grown);
      e.set(tokEnd);
      tokEnd = e;
      const p = new Uint8Array(grown);
      p.set(tokPlain);
      tokPlain = p;
      keep = new Uint8Array(grown);
    }
    tokStart[count] = start;
    tokEnd[count] = i;
    tokPlain[count] = plain;
    count++;
  }
  if (n > 0 && isSpace(input.charCodeAt(n - 1))) normalized = false;
  const e = ++epoch;
  let dropped = false;
  for (let t = count - 1; t >= 0; t--) {
    const raw = input.slice(tokStart[t], tokEnd[t]);
    const entry = tokPlain[t] ? (entries.get(raw) ?? null) : lookup(raw);
    if (entry === null) {
      keep[t] = 1;
      continue;
    }
    const base = entry.group * stride;
    if (base + stride > stamps.length) {
      const grown = new Int32Array(Math.max(stamps.length * 2, base + stride * 16));
      grown.set(stamps);
      stamps = grown;
    }
    if (stamps[base + entry.shape] === e) {
      keep[t] = 0;
      dropped = true;
      continue;
    }
    keep[t] = 1;
    const erases = entry.erases;
    for (let k = 0; k < erases.length; k++) stamps[base + erases[k]!] = e;
  }
  if (!dropped && normalized) return input;
  let out = "";
  for (let t = 0; t < count; ) {
    if (!keep[t]) {
      t++;
      continue;
    }
    const start = tokStart[t]!;
    let end = tokEnd[t]!;
    let u = t + 1;
    while (u < count && keep[u] && tokStart[u] === end + 1 && input.charCodeAt(end) === 32) end = tokEnd[u++]!;
    out = out ? out + " " + input.slice(start, end) : input.slice(start, end);
    t = u;
  }
  return out;
};

/** Drops every class erased by a single later class with the same variants. */
export const merge = (input: string): string => {
  let merged = cache[input];
  if (merged !== undefined) return merged;
  const hash = sampleHash(input);
  const slot = (hash & (DOOR_SIZE - 1)) + doorBase;
  const seen = door[slot] === (hash ^ doorEpoch) || door[slot ^ DOOR_SIZE] === (hash ^ (doorEpoch - 1));
  if (seen) {
    merged = prevCache[input];
    if (merged !== undefined) return (cache[input] = merged);
  }
  merged = mergeUncached(input);
  if (seen) {
    cache[input] = merged;
    if (++cacheCount > CACHE_SIZE) {
      cacheCount = 0;
      prevCache = cache;
      cache = Object.create(null);
      rotateDoor();
    }
  } else {
    door[slot] = hash ^ doorEpoch;
    if (++doorMarks > DOOR_SIZE) rotateDoor();
  }
  return merged;
};
