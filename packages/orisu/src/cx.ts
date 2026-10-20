export type ClassDictionary = Record<string, any>;
export type ClassArray = ClassValue[];
export type ClassValue = ClassArray | ClassDictionary | string | number | bigint | boolean | null | undefined;

export const toClassName = (value: ClassValue): string => {
  if (typeof value === "string") return value;
  if (typeof value === "number") return "" + value;
  if (!value || typeof value !== "object") return "";
  let out = "";
  if (Array.isArray(value)) {
    for (let i = 0; i < value.length; i++) {
      const item = value[i];
      if (!item) continue;
      const name = typeof item === "string" ? item : toClassName(item);
      if (name) out = out ? out + " " + name : name;
    }
    return out;
  }
  for (const key in value) {
    if (value[key]) out = out ? out + " " + key : key;
  }
  return out;
};

/** clsx-compatible class concatenation. */
export function cx(...inputs: ClassValue[]): string;
export function cx(): string {
  let out = "";
  for (let i = 0; i < arguments.length; i++) {
    const item = arguments[i] as ClassValue;
    if (!item) continue;
    const name = typeof item === "string" ? item : toClassName(item);
    if (name) out = out ? out + " " + name : name;
  }
  return out;
}
