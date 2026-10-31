# Benchmarks

orisu vs Tailwind 4.3.3 on the [shadcn/ui](https://github.com/shadcn-ui/ui) docs at a pinned commit: 4893 files,
18861 candidates, 512 KB of CSS. Before measuring, the bench checks that both find the same candidates and produce
byte-identical CSS. Apple M3, Node 26.

```sh
cargo xtask bench-tailwind [scenario...]
```

Instructions and time are per iteration, each the minimum over 5 runs. Instructions are CPU instructions retired, which
are more stable than wall time. Memory is the process's peak RSS, Node included.

## Compiler

| Scenario               | Instructions, Tailwind | orisu |      | Time, Tailwind |    orisu | Memory, Tailwind |    orisu |
| ---------------------- | ---------------------: | ----: | ---: | -------------: | -------: | ---------------: | -------: |
| compile                |                 249.3M | 28.3M | 8.8× |         8.6 ms |   1.8 ms |         104.2 MB |  69.2 MB |
| scan                   |                  4.86G | 3.43G | 1.4× |       139.0 ms |  73.7 ms |         143.0 MB | 106.2 MB |
| scan, cached           |                        | 453.2M |      |                |  16.7 ms |                  | 160.6 MB |
| build                  |                  1.68G | 167.2M | 10.0× |        62.3 ms |  12.2 ms |         237.2 MB | 156.2 MB |
| cold start             |                  7.11G | 3.59G | 2.0× |       224.3 ms |  87.0 ms |         234.1 MB | 123.2 MB |
| HMR                    |                 284.5M | 17.1M | 16.6× |        14.0 ms |   1.1 ms |         294.1 MB | 108.7 MB |
| HMR, no new class      |                   4.8M |  3.9M | 1.3× |         152 µs |   133 µs |         169.9 MB |  95.5 MB |
| canonicalize, cold     |                 33.95G | 1.17G | 29.1× |      1475.2 ms |  29.2 ms |         894.7 MB | 107.4 MB |
| canonicalize, warm     |                 227.5M |  1.4M | 166.9× |         9.8 ms |    69 µs |         422.6 MB |  86.3 MB |

Tailwind has no scan cache, so `scan, cached` has no counterpart.

## cn

The 1524 `cn(…)` calls (727 distinct) found in the project, against `clsx` 2.1.1 + `tailwind-merge` 3.7.0 (the classic
shadcn/ui `cn`) and [`cn`](https://github.com/shadcn-ui/cn) 0.2.6.

|                              | clsx + tailwind-merge |      cn | orisu/cn |
| ---------------------------- | --------------------: | ------: | -------: |
| Instructions per call        |                 34.8k |   14.0k |    10.2k |
| Time per call                |               1.48 µs | 0.51 µs |  0.26 µs |
| Instructions, each call once |                134.7M |  224.7M |   107.1M |
| Time, each call once         |                5.2 ms |  7.3 ms |   3.4 ms |
| Retained JS heap             |                1.4 MB |  881 KB |   812 KB |
| Bundle, minified             |               27.1 KB | 25.4 KB |  36.5 KB |
| Bundle, gzip                 |                8.5 KB | 10.4 KB |  14.0 KB |

"Per call" repeats the calls 20000 times, so caches are warm. "Each call once" is the first render: no cache hits.
Retained heap is measured after the warm run and includes caches and the merge table.

The orisu bundle includes the merge table the compiler generates for this project: 1927 class names, 31 KB minified
and 11 KB gzip. It grows with the number of classes the project uses. The `orisu/cn` code itself is 2.6 KB gzip.
