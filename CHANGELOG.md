# Changelog

All notable changes to this project are documented in this file.

This project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).
Write changes under `## [Unreleased]`, grouped by `### Breaking`, `### Added`, `### Changed`, `### Fixed` and similar headings.
The release workflow derives the version from them: `Breaking` or `Removed` bumps major (minor before 1.0),
`Added`, `Changed` or `Deprecated` bumps minor, anything else bumps patch.

## [Unreleased]

### Changed

- The mangler and the linter share one list of class attributes: the mangler now also reads `ngClass`, `[ngClass]`, `class:list`, `:class`, `v-bind:class` and `[class]`, and the language server also lints `*ClassName` attributes.
- A document outside every project directory gets a project only from its own workspace folder, and a non-file document gets none, instead of an arbitrary project.
- The language server takes incremental document changes and scans a document once per change, sharing the class regions between hover, completion, colors, code actions and diagnostics. Completion items are prepared once per project, and stylesheets are discovered in parallel. On the shadcn/ui `sidebar.tsx`, hover takes 122k instructions instead of 952k, completion 4.3M instead of 7.4M, colors 1.1M instead of 2.8M and diagnostics 2.3M instead of 3.4M; startup takes 52 ms instead of 58 ms.
- CSS `@import` resolution lives in Rust and is shared by Node and the language server. It tries a package's `exports` conditions in the package's own order, also searches `NODE_PATH`, and treats an unreadable `package.json` as missing instead of throwing.
- Every CLI subcommand's `--help` lists options in one format, with their defaults.
- `check` and the language server lint `classNames(…)` calls by default.
- The language server treats a stylesheet as a Tailwind entry only when it imports a Tailwind stylesheet (`@import "tailwindcss"` and its parts) or has `@tailwind utilities`, no longer matching packages like `tailwindcss-animate` by name. A file outside every project uses the project sharing the most parent directories with it.

### Fixed

- The scanner and the language server no longer crash on a tag ending in an unterminated quoted attribute, and the language server reads unquoted attributify values like `bg=red-500`, as the scanner does.
- The language server reports a `configFile` setting of the wrong type instead of ignoring it, and `compile` rejects `polyfills` beyond 8 bits instead of truncating them.
- The Vite plugin rejects a malformed saved mangle plan (odd `names`, mismatched `keys`/`values`, missing `namePrefix`) with an error instead of silently using it, and fills placeholders natively when no plan is saved.
- The language server reports the package version instead of `0.1.0`.
- An invalid language server setting is reported and ignored on its own instead of silently resetting every other setting.
- The Vite plugin fails with a clear message when the saved mangle plan cannot be read, instead of building as if there was none; only a missing plan or output file is ignored.
- `mangle` fails the build when a scanned file cannot be read, instead of treating it as empty and renaming the classes it uses.
- `mangleModule` expands variant groups itself when `variantGroups` is on, so hosts other than the Vite plugin get placeholders for the expanded classes.
- `ManglePlan.checkModule` rejects an unknown class fragment kind instead of reading it as `infix`.
- Listing the scanner's files or globs before the first scan no longer makes that scan return no candidates.
- An invalid `@source not` pattern is skipped instead of crashing the scan.
- Auto source detection no longer emits a bogus glob for package manifests.
- The language server skips every ignored content directory (`node_modules`, `.git`, build output and the like) when looking for stylesheets, not just `node_modules`.

## [0.4.0] - 2026-11-04

### Added

- The Vite plugin's `mangle` option renames classes to short names in builds: in the stylesheet, in the class positions of the project's scripts (`class`/`className` and `*ClassName` attributes, `cn`, `clsx`, `cx`, `classnames`, `twMerge`, `twJoin`, `cva`, `classList`, `setAttribute("class", …)`) and in the `orisu/cn` merge table, so `cn` keeps merging them. A class used anywhere else (another stylesheet, HTML, Markdown, a string outside a class position, dynamic parts like `` `text-${size}` ``) keeps its name, and so do the classes in `mangle.keep` and in `@source inline(…)`. On the shadcn/ui docs the minified stylesheet drops from 435.2 KB / 63.4 KB gzip to 372.7 KB / 57.8 KB. An SSR build in another process reads the plan the client build saves in the Vite cache directory. `mangle` is off with attributify and supports one utility stylesheet per build.
- `MANGLE_UNTRACKED_CLASS`, `MANGLE_NAME_MERGER` and `MANGLE_CLASS_COLLISION` fail a `mangle` build when a bundled module uses a renamed class where it cannot be renamed, imports `tailwind-merge` or `tailwind-variants`, or a stylesheet outside the build uses a renamed class or a short name.
- Compile errors carry a diagnostic code: `error.code` in JavaScript and the diagnostic code in the language server. `INVALID_STYLESHEET` covers the errors Tailwind reports, and `CIRCULAR_IMPORT` a stylesheet that imports itself, naming the chain of imports.
- `Scanner.scanNew()` returns only the candidates found since the previous scan.
- `Features` and `Polyfills` are generated from the compiler, so they can no longer drift from it.

### Changed

- Output and canonicalization follow Tailwind 4.3.4.
- The Vite plugin builds only the candidates found since the previous build of a stylesheet, keeps the scanned files in a set for HMR lookups, and lints only the files changed since the last diagnostics run.
- `compileAsync` loads all the missing `@import`s of an attempt together instead of restarting once per import.
- Adding a file to the scanner walks only the directories that lead to it instead of the whole source tree.
- The language server recompiles only the projects whose stylesheets changed, recompiles a project whose memory has grown far beyond its initial size, and finds misspelling suggestions among classes of similar length only.

### Fixed

- The scanner no longer turns invalid UTF-8 into strings without checking it.
- The scanner cache is invalidated when the scanner's code changes, not only on a new version, and concurrent writers no longer share a temporary file.
- Canonicalizing from Node and the language server no longer aliases the compiler it reads.
- A stylesheet that imports itself fails with `CIRCULAR_IMPORT` instead of hitting the depth limit, and the depth limit message no longer has a stray `)`.

## [0.3.0] - 2026-10-31

### Changed

- Scanning is faster and uses less memory. Each file is read, extracted and dropped in one pass, repeated lines are extracted once, and paths are matched by bytes: on the shadcn/ui docs a scan takes 73.7 ms instead of 106.1 ms and peaks at 106.2 MB instead of 122.8 MB. The scanner cache also keeps the directory walk, so a warm scan only checks the stats of walked directories and ignore files and takes 16.7 ms instead of 25.0 ms. It is now one file per set of sources (`scanner-<hash>.bin`), so CSS roots sharing a cache directory, as in Vite, no longer evict each other; caches from older versions are not read. `Scanner.files` no longer reads the files, and `orisu check` no longer extracts candidates to find them.

### Fixed

- `translate-*`, `translate-x-*`, `translate-y-*` and `translate-z-*` with a theme, fraction or arbitrary value (e.g. `-translate-x-1/2`) emit the `@property --tw-translate-*` rules again. Without them the unset axis was invalid and `translate` was dropped.

## [0.2.0] - 2026-10-30

### Changed

- `orisu/cn` ships a much smaller merge table: names are stored sorted and front-coded, and the erase lists are deduplicated. On the shadcn/ui docs the `cn` bundle drops from 114.1 KB minified / 17.5 KB gzip to 36.5 KB / 14.0 KB with identical output, and the table costs 596 KB of heap instead of 949 KB. A table module generated by an older version has to be generated again.

## [0.1.0] - 2026-10-20

### Added

- Initial release.
