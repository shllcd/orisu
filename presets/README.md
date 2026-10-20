# Presets

`tailwind-typography.css` and `tailwind-forms.css` replace the `@tailwindcss/typography` and `@tailwindcss/forms`
JavaScript plugins, which orisu does not run. Both are generated from the plugin output:

- `@tailwindcss/typography` 0.5.19
- `@tailwindcss/forms` 0.5.11

The only edit is an explicit `&` in nested selectors, which orisu keeps in its CSS output. To validate an update,
compile the upstream plugins with Tailwind 4.3.3 and compare the multiset of declarations, not the bytes: the nesting
syntax differs while the declarations must match exactly.

The files are copied to `packages/orisu/presets` by the package build; edit them here.
