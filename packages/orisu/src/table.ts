import type { MergeTableSource, NameTableSource } from "./merge.ts";
import type { MergeTable, NameTable } from "./native.ts";

/** One printable ASCII char holds the shared prefix length; a longer one is cut here and only costs bytes. */
const MAX_SHARED = 94;

const sharedPrefix = (a: string, b: string): number => {
  const max = Math.min(a.length, b.length, MAX_SHARED);
  let i = 0;
  while (i < max && a.charCodeAt(i) === b.charCodeAt(i)) i++;
  return i;
};

/** `values` as base-36 differences from the previous one (the first from `start`), an empty one for 0. */
const deltas = (values: readonly number[], start: number): string => {
  let previous = start;
  return values
    .map((value) => {
      const delta = value - previous;
      previous = value;
      return delta === 0 ? "" : delta.toString(36);
    })
    .join(",");
};

const encodeNames = (names: NameTable): NameTableSource => ({
  prefix: names.prefix,
  groups: names.groups.join(" "),
  opaque: names.opaque,
  runs: names.runs.map(({ group, shapes }) => group.toString(36) + ":" + deltas(shapes, 0)).join(";"),
  holes: deltas(names.holes, -1),
});

/** Shrinks a merge table to what `registerMergeTable` reads: sorted, front-coded names, one shape string, distinct erase lists. */
export const encodeMergeTable = (table: MergeTable): MergeTableSource => {
  const entries: [string, number][] = table.keys.map((key, i) => [key, table.values[i]!]);
  entries.sort((a, b) => (a[0] < b[0] ? -1 : 1));
  const lists: number[][] = [];
  const indices = new Map<string, number>();
  let keys = "";
  let values = "";
  let previous = "";
  for (let i = 0; i < entries.length; i++) {
    const [key, shape] = entries[i]!;
    const shared = sharedPrefix(key, previous);
    keys += String.fromCharCode(32 + shared) + key.slice(shared) + "\n";
    values += (i > 0 ? "," : "") + shape;
    previous = key;
  }
  const alias = table.erases.map((list) => {
    const id = list.join(",");
    let at = indices.get(id);
    if (at === undefined) {
      at = lists.length;
      indices.set(id, at);
      lists.push(list);
    }
    return at;
  });
  const source: MergeTableSource = { prefix: table.prefix ?? null, keys, values, erases: lists, alias: alias.join(",") };
  if (table.names) source.names = encodeNames(table.names);
  return source;
};
