# Benchmarks

orisu vs Tailwind 4.3.4 on the [shadcn/ui](https://github.com/shadcn-ui/ui) docs at a pinned commit: 4893 files,
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
| compile                |                 282.8M |  30.3M |  9.3× |         9.0 ms |  1.7 ms |  5.3× |         108.7 MB |  74.1 MB | 1.5× |
| scan                   |                  4.76G |  3.35G |  1.4× |       133.6 ms | 72.4 ms |  1.8× |         146.1 MB | 106.8 MB | 1.4× |
| scan, cached           |                        | 355.9M |       |                | 16.7 ms |       |                  | 157.8 MB |      |
| build                  |                  1.78G | 172.5M | 10.3× |        66.9 ms | 12.7 ms |  5.3× |         240.3 MB | 165.5 MB | 1.5× |
| cold start             |                  7.17G |  3.56G |  2.0× |       222.0 ms | 88.2 ms |  2.5× |         244.4 MB | 125.9 MB | 1.9× |
| HMR                    |                 345.3M |  16.7M | 20.7× |        17.9 ms |  1.1 ms | 16.3× |         311.1 MB | 110.8 MB | 2.8× |
| HMR, no new class      |                   4.7M |   3.4M |  1.4× |         149 µs |  140 µs |  1.1× |         175.4 MB |  98.1 MB | 1.8× |
| canonicalize, cold     |                 24.52G |  1.21G | 20.2× |      1145.1 ms | 27.7 ms | 41.3× |         963.6 MB | 112.5 MB | 8.6× |
| canonicalize, warm     |                  95.7M |   1.3M | 72.8× |         4.0 ms |   68 µs | 58.8× |         467.6 MB |  86.5 MB | 5.4× |

Tailwind has no scan cache, so `scan, cached` has no counterpart.

## Language server

`orisu-lsp` against `@tailwindcss/language-server` 0.16.0 with the project's Tailwind 4.3.4, both driven over stdio by the
same client on the same project. Requests target `registry/new-york-v4/ui/sidebar.tsx`.

```sh
cargo xtask bench-lsp [scenario...]
```

| Scenario           | Instructions, Tailwind |  orisu |        | Time, Tailwind |   orisu |       | Memory, Tailwind |   orisu |      |
| ------------------ | ---------------------: | -----: | -----: | -------------: | ------: | ----: | ---------------: | ------: | ---: |
| startup            |                 47.42G |  2.34G |  20.3× |      2646.0 ms | 52.0 ms | 50.9× |         619.4 MB | 72.8 MB | 8.5× |
| completion         |                 404.4M |   4.3M |  95.0× |        28.9 ms |  1.1 ms | 25.9× |         675.6 MB | 75.1 MB | 9.0× |
| completion resolve |                   1.4M | 128.2k |  10.6× |         180 µs |   94 µs |  1.9× |         628.0 MB | 74.3 MB | 8.5× |
| hover              |                  65.5M | 122.3k | 535.6× |         2.6 ms |  111 µs | 23.6× |         630.3 MB | 71.0 MB | 8.9× |
| diagnostics        |                 172.2M |   2.3M |  74.0× |        11.8 ms |  865 µs | 13.6× |         619.0 MB | 73.2 MB | 8.5× |
| colors             |                  66.0M |   1.1M |  60.5× |         3.3 ms |  351 µs |  9.4× |         630.8 MB | 72.8 MB | 8.7× |

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
| Instructions per call        |                 34.7k |   13.8k |    10.5k |
| Time per call                |               1.48 µs | 0.51 µs |  0.26 µs |
| Instructions, each call once |                136.5M |  227.7M |   103.0M |
| Time, each call once         |                5.2 ms |  7.4 ms |   3.2 ms |
| Retained JS heap             |                1.4 MB |  881 KB |   809 KB |
| Bundle, minified             |               27.1 KB | 25.4 KB |  37.2 KB |
| Bundle, gzip                 |                8.5 KB | 10.4 KB |  14.4 KB |

"Per call" repeats the calls 20000 times, so caches are warm. "Each call once" is the first render: no cache hits.
Retained heap is measured after the warm run and includes caches and the merge table.

The orisu bundle includes the merge table the compiler generates for this project: 1927 class names, 32.1 KB minified
and 12.1 KB gzip. It grows with the number of classes the project uses. The `orisu/cn` code itself is 2.3 KB gzip.
