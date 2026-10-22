# Editors

`orisu-lsp` is a language server for orisu and Tailwind CSS 4 projects: completions, hover, diagnostics,
quick fixes, `@theme` colors and class sorting. It speaks LSP over stdio, so any client that can launch a
command works. Every guide below replaces the Tailwind CSS language server with `orisu-lsp`, so class
tooling comes from orisu and the old server does not fight it.

## Install the server

From a checkout:

```sh
cargo build --release -p orisu_lsp   # target/release/orisu-lsp
```

From a project that already depends on the `orisu` package:

```sh
npx orisu lsp                        # or: node node_modules/orisu/dist/cli.js lsp
```

Put `orisu-lsp` on `PATH` (or use its absolute path) and check it responds:

```sh
orisu-lsp --version
```

## Settings

The server reads its options from LSP `initializationOptions` and from `workspace/didChangeConfiguration`.
Both accept the same object, either bare or wrapped in `{ "orisu": … }`; the Tailwind IntelliSense shape
`{ "tailwindCSS": … }` is accepted too, so existing configs keep working.

```json
{
  "classAttributes": ["class", "className", "ngClass", "[ngClass]", "class:list", ":class", "v-bind:class", "[class]"],
  "classFunctions": ["cn", "cva", "cx", "clsx", "classnames", "twMerge", "twJoin", "tv", "tw"],
  "rootFontSize": 16,
  "configFile": ["./app/globals.css"],
  "lint": {
    "cssConflict": "warning",
    "suggestCanonicalClasses": "warning",
    "unknownClasses": "ignore",
    "invalidThemeKey": "warning"
  }
}
```

- `configFile` — CSS entry points relative to the workspace; discovered automatically when omitted.
- `lint` — `ignore`, `hint`, `info`, `warning` or `error` per rule.
- `classAttributes` / `classFunctions` — where utility classes are looked for.

The server attaches to HTML, CSS, JavaScript, JSX, TypeScript, TSX, Vue, Svelte, Astro, Markdown and MDX.

## Zed

Zed's built-in Tailwind CSS support already registers a server for all of those languages, so the simplest
route is to keep it and point its binary at orisu. In your Zed `settings.json` (or a project `.zed/settings.json`):

```json
{
  "lsp": {
    "tailwindcss-language-server": {
      "binary": {
        "path": "/absolute/path/to/orisu-lsp",
        "arguments": []
      },
      "settings": {
        "orisu": {
          "rootFontSize": 16,
          "configFile": ["./app/globals.css"]
        }
      }
    }
  }
}
```

Zed restarts the server when this setting changes. Keep the Tailwind CSS extension enabled: it decides which
languages the server is started for. Disabling it with `"languages": { "HTML": { "language_servers": ["!tailwindcss-language-server"] } }`
turns orisu off as well.

`orisu-lsp` does not need the `tailwindcss` npm package installed, so a project works even when Tailwind
itself is only compiled by orisu.

## Neovim

With Neovim 0.11+ and built-in LSP, or with `nvim-lspconfig` for the filetypes:

```lua
vim.lsp.config("orisu", {
  cmd = { "orisu-lsp" },
  filetypes = { "html", "css", "javascript", "javascriptreact", "typescript", "typescriptreact", "vue", "svelte", "astro", "markdown", "mdx" },
  root_markers = { "package.json", "vite.config.ts", "tailwind.config.js", ".git" },
  init_options = {
    orisu = {
      rootFontSize = 16,
      configFile = { "./app/globals.css" },
    },
  },
})
vim.lsp.enable("orisu")
```

To replace the Tailwind server from `nvim-lspconfig` rather than run both, remove `tailwindcss` from
`vim.lsp.enable` (or disable it in `lspconfig`) and keep only `orisu`.

## Helix

`languages.toml`:

```toml
[language-server.orisu]
command = "orisu-lsp"
[language-server.orisu.config]
rootFontSize = 16
configFile = ["./app/globals.css"]

[[language]]
name = "html"
language-servers = ["orisu"]
```

Repeat the `[[language]]` block with `name` set to `css`, `javascript`, `typescript`, `tsx`, `vue`, `svelte`,
`astro`, `markdown` as needed. `language-servers` replaces the default list, so the Tailwind server no longer runs.

## Other clients

Start `orisu-lsp` on stdio with no arguments and pass your options as `initializationOptions`. The server
supports full document sync, completion (with resolve), hover, quick fixes and rewrite actions, and
document colors. It asks for `workspace/didChangeWatchedFiles` registration when the client supports it, so
editing CSS or theme files reloads the design system without a restart.
