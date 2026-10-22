# Project rules

Talk to the user in Russian, with short progress updates.

## Code

- No `//` comments except `// SAFETY:`, license headers and lint suppressions with a reason. Doc comments only where the signature doesn't express the contract.
- No dead code or TODOs.
- Don't commit tests. Correctness is checked byte-for-byte against Tailwind 4.3.3: `cargo xtask conformance <dir>`.
- Rust: `cargo fmt` (width 140), `cargo clippy --workspace` with no warnings.
- Crate dependencies point down only: `allocator → ast → parser → theme/candidate → registry → compiler → …`. Adapters (`compat_tailwind`, `lsp`, `napi`) depend on the core, never the reverse.
- Use oxc as a reference for parser structure and Tailwind for behavior, but copy neither blindly. Don't reproduce Tailwind bugs (e.g. canonicalize output depending on call order).

## Generated files

- Diagnostics live only in `crates/orisu_diagnostics/src/catalog.rs`. After editing, run `cargo xtask codegen`; `--check` catches stale output.
- `packages/orisu/presets` is copied from `presets/` by `npm run build`; don't edit it by hand.

## Workflow

- Don't edit or revert other people's changes; take files as they are on disk.
- Commit only when asked.
- Every optimization must keep output byte-identical (`bench --against` checks this).
- Record user-facing changes in `CHANGELOG.md` under `## [Unreleased]`; the `###` headings decide the release version bump.
