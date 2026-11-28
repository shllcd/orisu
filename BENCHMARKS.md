# Benchmarks

orisu vs Tailwind 4.3.5 on the [shadcn/ui](https://github.com/shadcn-ui/ui) docs at a pinned commit: 4893 files,
18861 candidates, 512 KB of CSS. Before measuring, the bench checks that both find the same candidates and produce
byte-identical CSS. Apple M3, Node 26.

```sh
cargo xtask bench-tailwind [scenario...]
```

Instructions and time are per iteration, each the minimum over 5 runs. Instructions are CPU instructions retired, which
are more stable than wall time. Memory is the process's peak RSS, Node included.

## Compiler

| Scenario               | Instructions, Tailwind |  orisu |       | Time, Tailwind |   orisu |       | Memory, Tailwind |    orisu |      |
| ---------------------- | ---------------------: | -----: | ----: | -------------: | ------: | ----: | ---------------: | -------: | ---: |
| compile                |                 267.3M |  26.5M | 10.1× |         9.0 ms |  1.6 ms |  5.6× |         105.0 MB |  72.4 MB | 1.5× |
| scan                   |                  4.76G |  3.27G |  1.5× |       145.1 ms | 70.2 ms |  2.1× |         146.3 MB | 106.9 MB | 1.4× |
| scan, cached           |                        | 348.8M |       |                | 16.4 ms |       |                  | 155.1 MB |      |
| build                  |                  1.80G | 169.9M | 10.6× |        67.5 ms | 12.6 ms |  5.4× |         239.3 MB | 163.3 MB | 1.5× |
| cold start             |                  7.17G |  3.48G |  2.1× |       225.9 ms | 86.0 ms |  2.6× |         231.6 MB | 146.8 MB | 1.6× |
| HMR                    |                 343.4M |  14.2M | 24.2× |        17.7 ms |  797 µs | 22.2× |         313.9 MB | 110.8 MB | 2.8× |
| HMR, no new class      |                   4.9M | 295.9k | 16.6× |       141.6 µs | 12.7 µs | 11.1× |         172.4 MB |  97.4 MB | 1.8× |
| canonicalize, cold     |                 24.01G |  1.20G | 20.0× |      1087.3 ms | 27.3 ms | 39.8× |         885.0 MB | 129.2 MB | 6.8× |
| canonicalize, warm     |                  81.5M |   1.2M | 67.9× |         3.6 ms | 66.4 µs | 54.2× |         465.4 MB |  97.2 MB | 4.8× |

Tailwind has no scan cache, so `scan, cached` has no counterpart.

## Language server

`orisu-lsp` against `@tailwindcss/language-server` 0.16.0 with the project's Tailwind 4.3.5, both driven over stdio by the
same client on the same project. Requests target `registry/new-york-v4/ui/sidebar.tsx`.

```sh
cargo xtask bench-lsp [scenario...]
```

| Scenario           | Instructions, Tailwind |  orisu |        | Time, Tailwind |   orisu |       | Memory, Tailwind |   orisu |      |
| ------------------ | ---------------------: | -----: | -----: | -------------: | ------: | ----: | ---------------: | ------: | ---: |
| startup            |                 46.92G |  2.30G |  20.4× |      2578.7 ms | 51.2 ms | 50.4× |         621.2 MB | 87.6 MB | 7.1× |
| completion         |                 502.5M |   4.0M | 125.6× |        29.3 ms |  508 µs | 57.7× |         659.5 MB | 86.9 MB | 7.6× |
| completion resolve |                 593.2k |  96.3k |   6.2× |       118.3 µs | 43.9 µs |  2.7× |         509.6 MB | 85.0 MB | 6.0× |
| hover              |                  55.2M | 99.7k | 553.7× |         2.8 ms | 29.6 µs | 94.7× |         547.0 MB | 85.7 MB | 6.4× |
| diagnostics        |                 204.5M |   2.1M |  97.4× |        12.8 ms |  477 µs | 26.8× |         530.5 MB | 87.8 MB | 6.0× |
| colors             |                  64.5M |   1.0M |  64.5× |         3.6 ms |  299 µs | 12.0× |         618.4 MB | 86.8 MB | 7.1× |

Instructions and memory are the server process's own; time is the client's wait, response parsing included. `startup`
runs from spawning the server to the first non-empty completion list and counts the whole session; the other rows are
per request after warm-up, measured like the compiler rows. `diagnostics` sends a full-text change that adds a new class
and waits for the published diagnostics. Tailwind debounces diagnostics by 500 ms; the bench turns that off with its
`testMode` option so the row compares work, not the delay.

The servers don't answer identically. A completion at the start of a class lists 26614 items from Tailwind and the first
1000 from orisu, marked incomplete so the editor asks again as you type. orisu finds 52 colors in the document, Tailwind
none: the shadcn theme defines its colors through `var(…)`, which orisu resolves. After the edit, both report the same two
class conflicts; orisu suggests three canonical classes, Tailwind two.

## cn

The 1524 `cn(…)` calls (727 distinct) found in the project, against `clsx` 2.1.1 + `tailwind-merge` 3.7.0 (the classic
shadcn/ui `cn`) and [`cn`](https://github.com/shadcn-ui/cn) 0.2.6.

|                              | clsx + tailwind-merge |      cn | orisu/cn |
| ---------------------------- | --------------------: | ------: | -------: |
| Instructions per call        |                 34.6k |   13.9k |     9.8k |
| Time per call                |               1.48 µs | 0.51 µs |  0.26 µs |
| Instructions, each call once |                133.7M |  224.0M |   102.3M |
| Time, each call once         |                5.2 ms |  7.4 ms |   3.2 ms |
| Retained JS heap             |                1.3 MB |  615 KB |   594 KB |
| Bundle, minified             |               27.1 KB | 25.4 KB |  36.5 KB |
| Bundle, gzip                 |                8.5 KB | 10.4 KB |  14.1 KB |

"Per call" repeats the calls 20000 times, so caches are warm. "Each call once" is the first render: no cache hits.
Retained heap is measured after the warm run and includes caches and the merge table.

The orisu bundle includes the merge table the compiler generates for this project: 1927 class names, 32.1 KB minified
and 12.1 KB gzip. It grows with the number of classes the project uses. The `orisu/cn` code itself is 2.3 KB gzip.
