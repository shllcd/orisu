// Inlined from Vite (https://github.com/vitejs/vite) by way of @tailwindcss/node,
// MIT License, Copyright (c) 2019-present, VoidZero Inc. and Vite contributors.

import path from "node:path";

const cssUrlRE = /(?<!@import\s+)(?<=^|[^\w\-\u0080-￿])url\((\s*('[^']+'|"[^"]+")\s*|[^'")]+)\)/;
const cssImageSetRE = /(?<=image-set\()((?:[\w-]{1,256}\([^)]*\)|[^)])*)(?=\))/;
const cssNotProcessedRE = /(?:gradient|element|cross-fade|image)\(/;
const dataUrlRE = /^\s*data:/i;
const externalRE = /^([a-z]+:)?\/\//;
const functionCallRE = /^[A-Z_][.\w-]*\(/i;
const imageCandidateRE =
  /(?:^|\s)(?<url>[\w-]+\([^)]*\)|"[^"]*"|'[^']*'|[^,]\S*[^,])\s*(?:\s(?<descriptor>\w[^,]+))?(?:,|$)/g;
const nonEscapedDoubleQuoteRE = /(?<!\\)"/g;
const escapedSpaceCharactersRE = /(?: |\\t|\\n|\\f|\\r)+/g;
const declarationRE = /(?:"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|[^;{}"'])+/g;

export const normalizePath = (original: string): string => {
  if (original === "\\" || original === "/") return "/";
  if (original.length <= 1) return original;
  let input = original;
  let prefix = "";
  if (input.length > 4 && input[3] === "\\" && (input[2] === "?" || input[2] === ".") && input.startsWith("\\\\")) {
    input = input.slice(2);
    prefix = "//";
  }
  const segments = input.split(/[/\\]+/);
  if (segments[segments.length - 1] === "") segments.pop();
  const normalized = prefix + segments.join("/");
  if (original.startsWith("\\\\") && normalized.startsWith("/") && !normalized.startsWith("//")) {
    return "/" + normalized;
  }
  return normalized;
};

type Replacer = (url: string) => string;

const replaceAll = (input: string, re: RegExp, replacer: (match: RegExpExecArray) => string): string => {
  let match: RegExpExecArray | null;
  let remaining = input;
  let out = "";
  while ((match = re.exec(remaining))) {
    out += remaining.slice(0, match.index) + replacer(match);
    remaining = remaining.slice(match.index + match[0].length);
  }
  return out + remaining;
};

const skip = (url: string): boolean =>
  externalRE.test(url) || dataUrlRE.test(url) || !/[.a-zA-Z0-9_]/.test(url[0]!) || functionCallRE.test(url);

const replaceUrl = (raw: string, matched: string, replacer: Replacer): string => {
  let wrap = "";
  const first = raw[0];
  if (first === '"' || first === "'") {
    wrap = first;
    raw = raw.slice(1, -1);
  }
  if (skip(raw)) return matched;
  let url = replacer(raw);
  if (wrap === "" && url !== encodeURI(url)) wrap = '"';
  if (wrap === "'" && url.includes("'")) wrap = '"';
  if (wrap === '"' && url.includes('"')) url = url.replace(nonEscapedDoubleQuoteRE, '\\"');
  return `url(${wrap}${url}${wrap})`;
};

const rewriteCssUrls = (css: string, replacer: Replacer): string =>
  replaceAll(css, cssUrlRE, ([matched, raw]) => replaceUrl(raw!.trim(), matched, replacer));

const rewriteImageSet = (css: string, replacer: Replacer): string =>
  replaceAll(css, cssImageSetRE, ([, raw]) =>
    Array.from(
      raw!
        .trim()
        .replace(escapedSpaceCharactersRE, " ")
        .replace(/\r?\n/, "")
        .replace(/,\s+/, ", ")
        .replaceAll(/\s+/g, " ")
        .matchAll(imageCandidateRE),
      ({ groups }) => ({ url: groups?.url?.trim() ?? "", descriptor: groups?.descriptor?.trim() ?? "" }),
    )
      .filter(({ url }) => !!url)
      .map(({ url, descriptor }) => {
        const rewritten = cssUrlRE.test(url)
          ? rewriteCssUrls(url, replacer)
          : cssNotProcessedRE.test(url)
            ? url
            : replaceUrl(url, url, replacer);
        return rewritten + (descriptor ? ` ${descriptor}` : "");
      })
      .join(", "),
  );

/** Rewrites relative `url(…)`s of a stylesheet loaded from `base` so they resolve from `root`. */
export const rewriteUrls = (css: string, base: string, root: string): string => {
  if (!css.includes("url(") && !css.includes("image-set(")) return css;
  const replacer = (url: string): string => {
    if (url[0] === "/") return url;
    const relative = path.posix.relative(normalizePath(root), path.posix.join(normalizePath(base), url));
    return relative.startsWith(".") ? relative : "./" + relative;
  };
  return css.replace(declarationRE, (chunk) =>
    cssImageSetRE.test(chunk)
      ? rewriteImageSet(chunk, replacer)
      : cssUrlRE.test(chunk)
        ? rewriteCssUrls(chunk, replacer)
        : chunk,
  );
};
