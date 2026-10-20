// Argument caching adapted from shadcn-ui/cn (https://github.com/shadcn-ui/cn),
// MIT License, Copyright (c) 2026 shadcn.

import "#merge-table";
import { toClassName, type ClassValue } from "./cx.ts";
import { merge, mergeUncached, sampleHash, seenBefore } from "./merge.ts";

export { cx, toClassName, type ClassArray, type ClassDictionary, type ClassValue } from "./cx.ts";
export {
  cva,
  defineConfig,
  type AnyCX,
  type CVA,
  type CVAComponent,
  type CVAComponentShape,
  type CVAVariantShape,
  type CX,
  type CXInput,
  type CXOptions,
  type CXReturn,
  type DefineConfig,
  type DefineConfigOptions,
  type VariantProps,
} from "./cva.ts";
export { merge, registerMergeTable } from "./merge.ts";

const BUCKET_CAP = 64;
const BUCKET_MISS_CAP = 16;
const BUCKET_SKIP = 1024;
const ARG_GENERATION = 1000;
const CHURN_SIZE = 4096;

interface Entry {
  args: string[];
  result: string;
  next: Entry | null;
}

interface Bucket {
  entries: Entry[];
  at: number;
  misses: number;
  skip: number;
}

let argCache = new Map<string, Bucket>();
let prevArgCache = new Map<string, Bucket>();
let argCount = 0;
let lastHit: Entry | null = null;

let churnKey: string[] | null = null;
let churnOwn: string[] = [];
let churnVal: string[] = [];
let churnTick = 0;

const churnLookup = (key: string, own: string): string => {
  if (churnKey === null) {
    churnKey = new Array<string>(CHURN_SIZE).fill("");
    churnOwn = new Array<string>(CHURN_SIZE).fill("");
    churnVal = new Array<string>(CHURN_SIZE).fill("");
  }
  const way = sampleHash(key) & (CHURN_SIZE - 2);
  if (churnKey[way] === key && churnOwn[way] === own) return churnVal[way]!;
  if (churnKey[way | 1] === key && churnOwn[way | 1] === own) return churnVal[way | 1]!;
  const merged = merge(own ? own + " " + key : key);
  const slot = churnKey[way] === "" ? way : way | (churnKey[way | 1] === "" ? 1 : churnTick++ & 1);
  churnKey[slot] = key;
  churnOwn[slot] = own;
  churnVal[slot] = merged;
  return merged;
};

const matches = (entry: Entry, values: ArrayLike<ClassValue>): boolean => {
  const args = entry.args;
  let k = 0;
  for (let i = 0; i < values.length; i++) {
    const value = values[i];
    if (!value) continue;
    if (value !== args[k]) return false;
    k++;
  }
  return k === args.length;
};

const predict = (values: ArrayLike<ClassValue>): string | undefined => {
  const hit = lastHit;
  if (hit === null) return undefined;
  const next = hit.next;
  if (next !== null && matches(next, values)) {
    lastHit = next;
    return next.result;
  }
  if (hit !== next && matches(hit, values)) return hit.result;
  return undefined;
};

const resolve = (values: ClassValue[]): string => {
  const args: string[] = [];
  for (let i = 0; i < values.length; i++) {
    let value = values[i];
    if (!value) continue;
    if (typeof value !== "string") {
      value = toClassName(value);
      if (!value) continue;
    }
    args.push(value);
  }
  if (args.length === 0) return "";
  if (args.length === 1) return merge(args[0]!);
  const first = args[0]!;
  let bucket = argCache.get(first);
  if (bucket === undefined) {
    bucket = prevArgCache.get(first);
    if (bucket !== undefined) argCache.set(first, bucket);
  }
  if (bucket !== undefined) {
    if (bucket.skip > 0) {
      bucket.skip--;
      lastHit = null;
      return churnLookup(args.slice(1).join(" "), first);
    }
    for (const entry of bucket.entries) {
      if (matches(entry, args)) {
        bucket.misses = 0;
        if (lastHit !== null && lastHit !== entry) lastHit.next = entry;
        lastHit = entry;
        return entry.result;
      }
    }
  }
  const joined = args.join(" ");
  if (!seenBefore(joined)) return mergeUncached(joined);
  const entry: Entry = { args, result: merge(joined), next: null };
  if (bucket === undefined) {
    bucket = { entries: [], at: 0, misses: 0, skip: 0 };
    argCache.set(first, bucket);
  } else if (++bucket.misses > BUCKET_MISS_CAP) {
    bucket.misses = BUCKET_MISS_CAP;
    bucket.skip = BUCKET_SKIP;
    bucket.entries.length = 0;
    bucket.at = 0;
  }
  if (bucket.entries.length < BUCKET_CAP) bucket.entries.push(entry);
  else {
    bucket.entries[bucket.at] = entry;
    bucket.at = (bucket.at + 1) & (BUCKET_CAP - 1);
  }
  if (++argCount > ARG_GENERATION) {
    argCount = 0;
    prevArgCache = argCache;
    argCache = new Map();
  }
  if (lastHit !== null && lastHit !== entry) lastHit.next = entry;
  lastHit = entry;
  return entry.result;
};

/** `merge(cx(...inputs))`, with repeated argument tuples answered without re-joining. */
export function cn(...inputs: ClassValue[]): string;
export function cn(): string {
  const n = arguments.length;
  if (n === 1) {
    const value = arguments[0] as ClassValue;
    if (typeof value === "string") return merge(value);
    return Array.isArray(value) ? resolve(value.slice()) : merge(toClassName(value));
  }
  const predicted = predict(arguments);
  if (predicted !== undefined) return predicted;
  const values: ClassValue[] = new Array(n);
  for (let i = 0; i < n; i++) values[i] = arguments[i];
  return resolve(values);
}
