// API and semantics ported from cva 1.0.0-beta.12 (https://github.com/joe-bell/cva),
// Apache License 2.0, Copyright 2022-present Joe Bell. Memoization ported from shadcn-ui/cn
// (https://github.com/shadcn-ui/cn/pull/19), MIT License, Copyright (c) shadcn.

import { cx as clsx, type ClassArray, type ClassDictionary, type ClassValue } from "./cx.ts";

export type { ClassArray, ClassDictionary, ClassValue };

type Uninferred<T> = [T][T extends any ? 0 : never];

export type StringToBoolean<T> = T extends "true" | "false" ? boolean : T;

export type InternalVariantKey = `_${string}`;

export type InternalOnlyWarning = "cva's generic parameters are restricted to internal use only.";

type CVAComponentConfigBase<T extends ClassValue = ClassValue> = { base?: T };

type CVAVariantSchema<V> = { [Variant in keyof V]?: StringToBoolean<keyof V[Variant]> | undefined };

type CVACompoundVariantSchema<V> = {
  [Variant in keyof V]?: StringToBoolean<keyof V[Variant]> | StringToBoolean<keyof V[Variant]>[] | undefined;
};

type CVACompoundVariants<V, T extends ClassValue> = (CVACompoundVariantSchema<V> & CVAClassProp<T>)[];

type CVAClassProp<T extends ClassValue = ClassValue> = { class?: T; className?: never } | { class?: never; className?: T };

export type CVAComponentConfig<
  Config,
  Variants,
  ComposedSingle extends CVAComponentShape | undefined = CVAComponentShape | undefined,
  ComposedList extends readonly CVAComponentShape[] = readonly CVAComponentShape[],
  T extends ClassValue = ClassValue,
  Merged = Variants,
> = Config & { composes?: ComposedSingle | readonly [...ComposedList] } & (Variants extends Record<
    string,
    Record<string, T>
  >
    ? CVAComponentConfigBase<T> & { variants?: Variants & { __proto__?: never } }
    : CVAComponentConfigBase<T> & { variants?: never }) &
  ([keyof Merged] extends [never]
    ? { compoundVariants?: never; defaultVariants?: never }
    : {
        compoundVariants?: CVACompoundVariants<Uninferred<Merged>, T>;
        defaultVariants?: CVAVariantSchema<Uninferred<Merged>>;
      });

/** Any function usable as a `cx` concatenator. */
export type AnyCX = (...inputs: any[]) => string;

type CXInputs<TCX extends AnyCX> = TCX extends (...inputs: readonly [...infer Inputs]) => string ? Inputs : never;

type CXInputElement<TCX extends AnyCX> = CXInputs<TCX>[number];

type CXHasSafeArity<TCX extends AnyCX> =
  CXInputs<TCX> extends infer Inputs
    ? Inputs extends readonly unknown[]
      ? Inputs extends []
        ? true
        : [] extends Inputs
          ? number extends Inputs["length"]
            ? true
            : false
          : false
      : false
    : false;

type CXConstraint<TCX extends AnyCX> =
  false extends CXHasSafeArity<TCX>
    ? "cva's cx must accept zero arguments and an unbounded rest parameter."
    : [TCX] extends [(...inputs: (string | CXInput<TCX>)[]) => string]
      ? unknown
      : "cva's cx must accept its inferred class values and composed strings.";

/** The class value type a concatenator accepts, inferred from its parameters. */
export type CXInput<TCX extends AnyCX> =
  CXInputElement<TCX> extends infer P
    ? 0 extends 1 & P
      ? ClassValue
      : [P] extends [never]
        ? ClassValue
        : [P] extends [ClassValue]
          ? P
          : [Extract<P, ClassValue>] extends [never]
            ? ClassValue
            : Extract<P, ClassValue>
    : ClassValue;

type OmitUndefined<T> = T extends undefined ? never : T;

type UnionToIntersection<U> = (U extends any ? (k: U) => void : never) extends (k: infer I) => void ? I : never;

type ComposedTuple<S extends CVAComponentShape | undefined, L extends readonly CVAComponentShape[]> = [S] extends [
  CVAComponentShape,
]
  ? [S]
  : L;

type MergedVariants<T extends readonly unknown[]> = UnionToIntersection<
  { [K in keyof T]: T[K] extends { config: { variants?: infer V extends CVAVariantShape } } ? V : never }[number]
>;

type RightMerge<A, B> = { [K in keyof A | keyof B]: K extends keyof B ? B[K] : K extends keyof A ? A[K] : never };

type DefaultsOf<Component> = Component extends { config: { defaultVariants?: infer D } }
  ? D extends undefined
    ? {}
    : D
  : {};

type MergedDefaultVariants<T extends readonly unknown[]> = T extends readonly [infer Head, ...infer Rest]
  ? RightMerge<DefaultsOf<Head>, MergedDefaultVariants<Rest>>
  : {};

export type VariantProps<Component extends (...args: any) => any> = Omit<
  OmitUndefined<Parameters<Component>[0]>,
  "class" | "className" | InternalVariantKey
>;

export interface CX<T extends ClassValue = ClassValue> {
  (...inputs: T[]): string;
}

export type CXOptions = Parameters<CX>;

export type CXReturn = ReturnType<CX>;

export type CVAVariantShape = Record<string, Record<string, ClassValue>>;

export interface CVAComponent<Config, Variants, T extends ClassValue = ClassValue> {
  (props?: Variants extends CVAVariantShape ? CVAVariantSchema<Variants> & CVAClassProp<T> : CVAClassProp<T>): string;
  /** The definition with `variants` and `defaultVariants` merged across `composes`. */
  config: Config;
}

export type CVAComponentShape = CVAComponent<any, any, any>;

type CVADefaultVariants<Config> = Config extends { defaultVariants?: infer D } ? D : {};

type AllVariants<
  Variants,
  ComposedSingle extends CVAComponentShape | undefined,
  ComposedList extends readonly CVAComponentShape[],
> = [ComposedSingle] extends [undefined]
  ? [ComposedList] extends [readonly []]
    ? Variants
    : Variants & MergedVariants<ComposedList>
  : Variants & MergedVariants<ComposedTuple<ComposedSingle, ComposedList>>;

export interface CVA<T extends ClassValue = ClassValue> {
  <
    _ extends InternalOnlyWarning,
    Config,
    Variants,
    ComposedSingle extends CVAComponentShape | undefined = undefined,
    ComposedList extends readonly CVAComponentShape[] = [],
  >(
    config: CVAComponentConfig<
      Config,
      Variants,
      ComposedSingle,
      ComposedList,
      T,
      AllVariants<Variants, ComposedSingle, ComposedList>
    >,
  ): CVAComponent<
    Omit<Config, "defaultVariants"> & {
      variants: AllVariants<Variants, ComposedSingle, ComposedList>;
      defaultVariants: Omit<
        MergedDefaultVariants<ComposedTuple<ComposedSingle, ComposedList>>,
        keyof CVADefaultVariants<Config>
      > &
        CVADefaultVariants<Config>;
    },
    AllVariants<Variants, ComposedSingle, ComposedList>,
    T
  >;
}

export interface DefineConfigOptions<TCX extends AnyCX = CX> {
  /**
   * The concatenator used by `cva` and `cx`. Composed outputs, `base`, matched variant and compound
   * values and `class`/`className` are passed through verbatim, one argument each.
   */
  cx: TCX & CXConstraint<TCX>;
}

export interface DefineConfig {
  <TCX extends AnyCX>(options: DefineConfigOptions<TCX>): { cx: CX<CXInput<TCX>>; cva: CVA<CXInput<TCX>> };
}

const MEMO_ROWS = 16;
const MEMO_NARROW_ROWS = 8;
const MEMO_MAX_ROW_WIDTH = 16;
const TABLE_MAX_SLOTS = 256;
const FAST_LANE_MAX_PROPS = 2;
const FAST_LANE_ROW_WIDTH = 4;

type Props = Record<string, unknown>;

interface Definition {
  base?: unknown;
  composes?: unknown;
  variants?: Record<string, Props>;
  defaultVariants?: Props;
  compoundVariants?: Props[];
}

type Component = ((input?: Props | null) => string) & { config: Props };

interface Compound {
  start: number;
  end: number;
  mask: number;
  matchesDefaults: boolean;
  class: unknown;
  className: unknown;
}

interface Memo {
  run: (props: Props) => string;
  names: readonly string[];
  name0: string;
  name1: string;
  fast: boolean;
  width: number;
  candidate: unknown[];
  values: unknown[];
  results: string[];
  ringRows: number;
  writtenRows: number;
  scanRows: number;
  nextRow: number;
  hitThisPass: boolean;
}

interface Table {
  names: readonly string[];
  states: Record<string, number>[];
  counts: number[];
  results: (string | null)[];
}

const EMPTY: Props = {};
const hasOwn = Object.prototype.hasOwnProperty;
const ownEnumerable = Object.prototype.propertyIsEnumerable;
const memoKeys = new WeakMap<object, readonly string[] | null>();

const filled = <T>(count: number, value: T): T[] => {
  const out: T[] = [];
  for (let i = 0; i < count; i++) out.push(value);
  return out;
};

const isReference = (value: unknown): boolean =>
  value !== null && (typeof value === "object" || typeof value === "function");

const falsyToString = (value: unknown): unknown =>
  typeof value === "boolean" ? `${value}` : value === 0 ? "0" : value;

const pushDefined = (out: unknown[], value: unknown): void => {
  if (value !== undefined) out.push(value);
};

const definedProps = (props: Props, seed: Props): Props => {
  let merged: Props = { ...seed };
  for (const key in props) {
    if (!hasOwn.call(props, key)) continue;
    const value = props[key];
    if (key === "class" || key === "className" || value === undefined) continue;
    if (key === "__proto__") merged = { ...merged, [key]: value };
    else merged[key] = value;
  }
  return merged;
};

const mergeInto = (merged: { variants: Record<string, Props>; defaults: Props }, source: Definition | undefined) => {
  const variants = source && source.variants;
  for (const key in variants) {
    if (!hasOwn.call(variants, key)) continue;
    const values = { ...merged.variants[key], ...variants[key] };
    if (key === "__proto__") merged.variants = { ...merged.variants, [key]: values };
    else merged.variants[key] = values;
  }
  merged.defaults = { ...merged.defaults, ...(source && source.defaultVariants) };
};

const compoundMatches = (compound: Compound, indexes: number[], selectors: unknown[], values: unknown[]): boolean => {
  for (let i = compound.start; i < compound.end; i++) {
    const selector = selectors[i];
    const value = values[indexes[i]!];
    if (Array.isArray(selector) ? !selector.includes(value) : value !== selector) return false;
  }
  return true;
};

const createMemo = (run: (props: Props) => string, names: readonly string[]): Memo => {
  const fast = names.length <= FAST_LANE_MAX_PROPS;
  const width = fast ? FAST_LANE_ROW_WIDTH : names.length + 2;
  return {
    run,
    names,
    name0: names[0] ?? "class",
    name1: names[1] ?? names[0] ?? "class",
    fast,
    width,
    candidate: filled<unknown>(width, undefined),
    values: filled<unknown>(MEMO_ROWS * width, undefined),
    results: filled(MEMO_ROWS, ""),
    ringRows: MEMO_ROWS,
    writtenRows: 0,
    scanRows: 0,
    nextRow: 0,
    hitThisPass: true,
  };
};

const createTable = (names: readonly string[], maps: unknown[]): Table | null => {
  const states: Record<string, number>[] = [];
  const counts: number[] = [];
  let slots = 1;
  for (let i = 0; i < names.length; i++) {
    const map = maps[i];
    if (!isReference(map)) return null;
    const byKey: Record<string, number> = Object.create(null);
    let count = 1;
    for (const key of Object.keys(map as object)) byKey[key] = count++;
    states.push(byKey);
    counts.push(count);
    slots *= count;
    if (slots > TABLE_MAX_SLOTS) return null;
  }
  return { names, states, counts, results: filled<string | null>(slots, null) };
};

const memoMiss = (m: Memo, props: Props): string => {
  const width = m.width;
  const candidate = m.candidate;
  const className = m.run(props);
  for (let i = 0; i < width; i++) {
    if (isReference(candidate[i])) return className;
  }
  const values = m.values;
  const row = m.nextRow;
  const start = row * width;
  for (let i = 0; i < width; i++) values[start + i] = candidate[i];
  m.results[row] = className;
  let next = row + 1;
  if (row === m.writtenRows) {
    m.writtenRows = next;
    m.scanRows = next;
  }
  if (next === m.ringRows) {
    const ringRows = m.hitThisPass ? MEMO_ROWS : MEMO_NARROW_ROWS;
    m.ringRows = ringRows;
    m.scanRows = m.writtenRows < ringRows ? m.writtenRows : ringRows;
    m.hitThisPass = false;
    next = 0;
  }
  m.nextRow = next;
  return className;
};

const fastMemo = (m: Memo, props: Props): string => {
  const first = props[m.name0];
  const second = props[m.name1];
  const extra = props.class;
  const extraName = props.className;
  const values = m.values;
  const rows = m.scanRows;
  for (let row = 0; row < rows; row++) {
    const start = row * FAST_LANE_ROW_WIDTH;
    if (
      first === values[start] &&
      second === values[start + 1] &&
      extra === values[start + 2] &&
      extraName === values[start + 3]
    ) {
      m.hitThisPass = true;
      return m.results[row]!;
    }
  }
  const candidate = m.candidate;
  candidate[0] = first;
  candidate[1] = second;
  candidate[2] = extra;
  candidate[3] = extraName;
  return memoMiss(m, props);
};

const wideMemo = (m: Memo, props: Props): string => {
  const width = m.width;
  const names = m.names;
  const count = names.length;
  const candidate = m.candidate;
  for (let i = 0; i < count; i++) candidate[i] = props[names[i]!];
  candidate[count] = props.class;
  candidate[count + 1] = props.className;
  const values = m.values;
  const rows = m.scanRows;
  for (let row = 0; row < rows; row++) {
    const start = row * width;
    let i = 0;
    while (i < width && candidate[i] === values[start + i]) i++;
    if (i === width) {
      m.hitThisPass = true;
      return m.results[row]!;
    }
  }
  return memoMiss(m, props);
};

const tableSlot = (t: Table, props: Props): number => {
  const names = t.names;
  let slot = 0;
  for (let i = 0; i < names.length; i++) {
    const value = props[names[i]!];
    let state = 0;
    if (value !== undefined && value !== null && value !== "" && value === value) {
      const type = typeof value;
      const key =
        type === "string" ? (value as string) : type === "boolean" || type === "number" ? String(value) : undefined;
      const found = key === undefined ? undefined : t.states[i]![key];
      if (found === undefined) return -1;
      state = found;
    }
    slot = slot * t.counts[i]! + state;
  }
  return slot;
};

const createComponent = (cxArray: (inputs: unknown[]) => string, config: Definition | undefined): Component => {
  const definition = config || (EMPTY as Definition);
  const composes = definition.composes;
  const base = definition.base;
  const children = (composes == null ? [] : Array.isArray(composes) ? composes.slice() : [composes]) as Component[];
  const childCount = children.length;

  const merged = { variants: {} as Record<string, Props>, defaults: {} as Props };
  for (let i = 0; i < childCount; i++) mergeInto(merged, children[i]!.config as Definition);
  mergeInto(merged, definition);
  const defaults = merged.defaults;

  const variantKeys: string[] = [];
  const variantMaps: Props[] = [];
  const localVariants = definition.variants;
  for (const key in localVariants) {
    if (!hasOwn.call(localVariants, key)) continue;
    variantKeys.push(key);
    variantMaps.push(localVariants[key]!);
  }
  const defaultClasses = variantKeys.map((key, i) => variantMaps[i]![falsyToString(defaults[key]) as string]);
  const variantCount = variantKeys.length;

  const keys = variantKeys.slice();
  const compounds: Compound[] = [];
  const indexes: number[] = [];
  const selectors: unknown[] = [];
  const compoundVariants = definition.compoundVariants;
  if (compoundVariants) {
    for (let i = 0; i < compoundVariants.length; i++) {
      const compound = compoundVariants[i]!;
      const start = indexes.length;
      let mask = 0;
      for (const key in compound) {
        if (!hasOwn.call(compound, key) || key === "class" || key === "className") continue;
        let index = keys.indexOf(key);
        if (index === -1) index = keys.push(key) - 1;
        indexes.push(index);
        selectors.push(compound[key]);
        mask |= 1 << index;
      }
      compounds.push({
        start,
        end: indexes.length,
        mask,
        matchesDefaults: false,
        class: compound.class,
        className: compound.className,
      });
    }
  }
  const defaultValues = keys.map((key) => defaults[key]);
  for (const compound of compounds) compound.matchesDefaults = compoundMatches(compound, indexes, selectors, defaultValues);
  const keyCount = keys.length;
  const compoundCount = compounds.length;

  const run = (props: Props): string => {
    let supplied = 0;
    const variantClasses: unknown[] = new Array(variantCount);
    const resolved: unknown[] = new Array(keyCount);
    for (let i = 0; i < keyCount; i++) {
      const key = keys[i]!;
      const own = compoundCount !== 0 && ownEnumerable.call(props, key);
      const value = i < variantCount || own ? props[key] : undefined;
      if (value !== undefined) supplied |= i < 31 ? 1 << i : -1;
      if (i < variantCount) {
        const variantKey = falsyToString(value);
        variantClasses[i] = variantKey ? variantMaps[i]![variantKey as string] : defaultClasses[i];
      }
      resolved[i] = own && value !== undefined ? value : defaultValues[i];
    }
    const out: unknown[] = [];
    if (childCount) {
      const forwarded = definedProps(props, defaults);
      for (let i = 0; i < childCount; i++) pushDefined(out, children[i]!({ ...forwarded }));
    }
    pushDefined(out, base);
    for (let i = 0; i < variantCount; i++) pushDefined(out, variantClasses[i]);
    for (let i = 0; i < compoundCount; i++) {
      const compound = compounds[i]!;
      let matched = compound.matchesDefaults;
      if (supplied & compound.mask) matched = compoundMatches(compound, indexes, selectors, resolved);
      if (matched) {
        pushDefined(out, compound.class);
        pushDefined(out, compound.className);
      }
    }
    pushDefined(out, props.class);
    pushDefined(out, props.className);
    return cxArray(out);
  };

  let memoNames: string[] | null = keys.slice();
  for (const child of children) {
    const childNames = typeof child === "function" ? memoKeys.get(child) : undefined;
    if (!childNames) {
      memoNames = null;
      break;
    }
    for (const name of childNames) if (!memoNames.includes(name)) memoNames.push(name);
  }
  if (memoNames !== null && memoNames.includes("__proto__")) memoNames = null;
  const memoizable = memoNames !== null && memoNames.length + 2 <= MEMO_MAX_ROW_WIDTH;

  let defaultResult: string | undefined;
  let memo: Memo | null = null;
  let table: Table | null | undefined;

  const component = ((input?: Props | null): string => {
    const props = (input || EMPTY) as Props;
    if (!memoizable) return run(props);
    if (props === EMPTY) return (defaultResult ??= run(EMPTY));
    if (table === undefined) {
      table = !childCount && !compoundCount && variantCount ? createTable(variantKeys, variantMaps) : null;
    }
    if (table !== null && props.class === undefined && props.className === undefined) {
      const slot = tableSlot(table, props);
      if (slot !== -1) return (table.results[slot] ??= run(props));
    }
    const proto = Object.getPrototypeOf(props);
    if (proto !== Object.prototype && proto !== null) return run(props);
    memo ??= createMemo(run, memoNames!);
    return memo.fast ? fastMemo(memo, props) : wideMemo(memo, props);
  }) as Component;

  memoKeys.set(component, memoizable ? memoNames : null);
  component.config = { ...(config as Props), variants: merged.variants, defaultVariants: defaults };
  return component;
};

/** Binds `cva` and `cx` to a concatenator such as `clsx`, `twMerge` or `cn`. */
export const defineConfig: DefineConfig = (options) => {
  const cxArray = (inputs: unknown[]): string => Reflect.apply(options.cx as AnyCX, options, inputs);
  const cx = (...inputs: unknown[]): string => cxArray(inputs.filter((input) => input !== undefined));
  const cva = (config: Definition | undefined) => createComponent(cxArray, config);
  return { cx, cva } as unknown as ReturnType<DefineConfig>;
};

export const { cva } = defineConfig({ cx: clsx });
