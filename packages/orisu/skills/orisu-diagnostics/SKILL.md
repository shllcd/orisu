---
name: orisu-diagnostics
description: Repair guide for orisu diagnostics. Use when a build, dev server, editor or `orisu check` prints a bracketed orisu code such as [UNKNOWN_UTILITY], or when diagnostics.jsonl contains orisu diagnostics.
---

# orisu diagnostics

<!-- Generated from crates/orisu_diagnostics/src/catalog.rs by `cargo xtask codegen`. Do not edit by hand. -->

Every diagnostic starts with its code in brackets, `[CODE]`, and says what was seen, why it is wrong and what to do.

## How to act

1. When the diagnostic has `fixes`, apply them: each fix is a list of exact byte edits (`start`, `end`, `text`) into the source that removes the diagnostic. `orisu check --fix` applies them all.
2. Otherwise follow the **Repair** of the code below.
3. Never suppress or work around an `error` or a `warn`; fix its cause.

## JSONL channel

With the Vite plugin option `orisu({ diagnostics: { jsonl: "path" } })`, `path` holds every current diagnostic as one JSON object per line, rewritten after each rebuild, with `code`, `severity`, `message`, `file`, `start`/`end` (`offset`, 1-based `line`, 0-based UTF-16 `column`), `labels`, `fixes`, `data` and `rendered`.

## Codes

## UNKNOWN_UTILITY

**Class that generates no CSS** · severity `error`

> `{class}` is not a utility, so it generates no CSS.

`data` keys: `class`, `suggestion`

Automatic fix: yes

No utility, `@utility` or arbitrary property matches the class, so the stylesheet has no rule for it. `data.class` is the class. When a known utility is a close spelling, the report carries a fix and `data.suggestion` is that utility.

**Repair:** Apply the fix when there is one. Otherwise check the spelling against the theme, declare the utility with `@utility`, or write an arbitrary value such as `w-[13px]`.

Before:

```html
<div class="flex itms-center"></div>
```

After:

```html
<div class="flex items-center"></div>
```

## UNKNOWN_VARIANT

**Unknown variant** · severity `error`

> `{variant}:` is not a variant, so `{class}` generates no CSS.

`data` keys: `class`, `variant`

Automatic fix: no

Every variant of a class must be known: built in, declared with `@custom-variant`, or arbitrary such as `[&>*]:`. An unknown one drops the whole class. `data.variant` is the variant and `data.class` the class.

**Repair:** Fix the spelling, declare the variant with `@custom-variant`, or write it as an arbitrary variant.

Before:

```html
<a class="hovr:underline"></a>
```

After:

```html
<a class="hover:underline"></a>
```

## UNSUPPORTED_PLUGIN

**JavaScript plugin or config** · severity `error`

> `@{directive} "{id}"` loads JavaScript, which orisu does not run.{hint}

`data` keys: `directive`, `hint`, `id`

Automatic fix: no

orisu compiles CSS without a JavaScript runtime, so `@plugin` and `@config` are rejected and the stylesheet does not build. `data.directive` is `plugin` or `config`, `data.id` the module, and `data.hint` names the CSS preset that replaces a known plugin, or is empty.

**Repair:** Import the preset named in the message instead of the plugin. Otherwise move the configuration into `@theme`, `@utility` and `@custom-variant`.

Before:

```css
@import "tailwindcss";
@plugin "@tailwindcss/typography";
```

After:

```css
@import "tailwindcss";
@import "orisu/presets/tailwind-typography.css";
```

## CONFLICTING_CLASSES

**Classes that set the same properties** · severity `warn`

> `{class}` sets the same CSS properties as {others}, so only one of them takes effect.

`data` keys: `class`, `others`

Automatic fix: no

Both classes apply under the same variants and write the same properties. The one later in the stylesheet wins regardless of order in the attribute. `data.class` is this class and `data.others` the conflicting ones, each in backticks.

**Repair:** Keep one of the classes. To let a later class override an earlier one at runtime, combine them with `cn` from `orisu/cn`.

Before:

```html
<div class="p-2 p-4"></div>
```

After:

```html
<div class="p-4"></div>
```

## INVALID_THEME_KEY

**Reference to a missing theme key** · severity `warn`

> `{key}` is defined neither in the theme nor in a stylesheet, so `{class}` reads an empty value.

`data` keys: `class`, `key`

Automatic fix: no

The class reads a custom property through `(--key)` or `var(--key)`. The key sits in a theme namespace such as `--color-*` or `--spacing-*`, yet `@theme` and the project's stylesheets never define it, which usually means a misspelling. `data.key` is the variable and `data.class` the class.

**Repair:** Fix the spelling, add the key under `@theme`, or define the property where the element can inherit it.

Before:

```html
<p class="text-(--color-brnd)"></p>
```

After:

```html
<p class="text-(--color-brand)"></p>
```

## NON_CANONICAL_CLASS

**Class with a shorter spelling** · severity `warn`

> `{class}` can be written as `{canonical}`.

`data` keys: `canonical`, `class`

Automatic fix: yes

The class generates the same CSS as a shorter or theme-based spelling. `data.class` is the class and `data.canonical` the replacement.

**Repair:** Apply the fix.

Before:

```html
<div class="w-[16px]"></div>
```

After:

```html
<div class="w-4"></div>
```
