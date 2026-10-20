use std::fmt::Write;

use crate::catalog::CATALOG;
use crate::render::placeholders;

const HEADER: &str = "---
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

With the Vite plugin option `orisu({ diagnostics: { jsonl: \"path\" } })`, `path` holds every current diagnostic as one JSON object per line, rewritten after each rebuild, with `code`, `severity`, `message`, `file`, `start`/`end` (`offset`, 1-based `line`, 0-based UTF-16 `column`), `labels`, `fixes`, `data` and `rendered`.

## Codes
";

pub fn render_skill() -> String {
    let mut out = String::from(HEADER);
    for entry in CATALOG {
        let _ =
            write!(out, "\n## {}\n\n**{}** · severity `{}`\n\n> {}\n\n", entry.name, entry.title, entry.severity.as_str(), entry.message);
        let mut keys: Vec<&str> = placeholders(entry.message).chain(entry.fix.into_iter().flat_map(placeholders)).collect();
        keys.sort_unstable();
        keys.dedup();
        out.push_str("`data` keys:");
        for (i, key) in keys.iter().enumerate() {
            out.push_str(if i == 0 { " `" } else { ", `" });
            out.push_str(key);
            out.push('`');
        }
        let _ = write!(
            out,
            "\n\nAutomatic fix: {}\n\n{}\n\n**Repair:** {}\n\nBefore:\n\n```{}\n{}```\n\nAfter:\n\n```{}\n{}```\n",
            if entry.fix.is_some() { "yes" } else { "no" },
            entry.explanation,
            entry.repair,
            entry.example.lang,
            entry.example.bad,
            entry.example.lang,
            entry.example.good
        );
    }
    out
}
