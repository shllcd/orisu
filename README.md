# orisu

A Tailwind CSS 4 engine written in Rust. The goal is a drop-in replacement: the same CSS byte for byte as
Tailwind 4.3.3, with less work on every step that matters while you are developing: compile, build, HMR,
class canonicalization and `cn`.

```sh
npm install orisu
```

```ts
// vite.config.ts
import orisu from "orisu/vite";

export default { plugins: [orisu()] };
```

Editor support is described in [EDITORS.md](EDITORS.md).

## cn

`orisu/cn` replaces `clsx` + `tailwind-merge`:

```ts
import { cn } from "orisu/cn";

cn("px-2 py-1", active && "px-4", { "text-white": active }); // "py-1 px-4 text-white"
```

tailwind-merge guesses class conflicts from a hand-written config. orisu takes them from the compiler: during the build
the Vite plugin generates a merge table from the CSS the project actually produces. Custom utilities, `@theme` values
and plugins merge correctly without extra configuration. Without Vite, `orisu build --merge-table <file>` writes the
same table as a module to import once.

`cx` and `cva` are exported from `orisu/cn` too.

## Benchmarks

On the shadcn/ui docs, with byte-identical CSS:

| Scenario           | Tailwind 4.3.3 |   orisu |
| ------------------ | -------------: | ------: |
| build              |        62.9 ms | 12.2 ms |
| HMR                |        14.4 ms |  1.1 ms |
| canonicalize, cold |      1560.7 ms | 30.2 ms |
| `cn` per call      |        1.51 µs | 0.29 µs |

The full numbers (instructions, memory, `cn` against shadcn's `cn`, bundle size) are in [BENCHMARKS.md](BENCHMARKS.md).

## License

MIT
