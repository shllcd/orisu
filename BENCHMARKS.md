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
| compile                |                 253.2M | 29.0M | 8.7× |         9.0 ms |   1.9 ms |         104.5 MB |  69.3 MB |
| scan                   |                  4.87G | 4.10G | 1.2× |       151.0 ms |  81.5 ms |         143.1 MB | 104.5 MB |
| scan, cached           |                        | 743.7M |      |                |  25.0 ms |                  | 151.0 MB |
| build                  |                  1.67G | 167.2M | 10.0× |        63.1 ms |  12.3 ms |         237.8 MB | 156.2 MB |
| cold start             |                  7.04G | 4.28G | 1.6× |       219.1 ms |  94.6 ms |         232.8 MB | 120.7 MB |
| HMR                    |                 287.0M | 18.8M | 15.3× |        15.1 ms |   1.1 ms |         275.3 MB | 106.6 MB |
| HMR, no new class      |                   5.0M |  3.7M | 1.3× |         149 µs |   136 µs |         170.2 MB |  95.1 MB |
| canonicalize, cold     |                 32.78G | 1.17G | 28.0× |      1475.7 ms |  29.6 ms |         821.7 MB | 106.6 MB |
| canonicalize, warm     |                 219.9M |  1.2M | 184.7× |         9.8 ms |    68 µs |         422.0 MB |  85.3 MB |

Tailwind has no scan cache, so `scan, cached` has no counterpart.

## cn

The 1524 `cn(…)` calls (727 distinct) found in the project, against `clsx` 2.1.1 + `tailwind-merge` 3.7.0 (the classic
shadcn/ui `cn`) and [`cn`](https://github.com/shadcn-ui/cn) 0.2.6.

|                              | clsx + tailwind-merge |      cn | orisu/cn |
| ---------------------------- | --------------------: | ------: | -------: |
| Instructions per call        |                 34.7k |   13.8k |    10.0k |
| Time per call                |               1.48 µs | 0.51 µs |  0.26 µs |
| Instructions, each call once |                139.2M |  224.1M |   105.4M |
| Time, each call once         |                5.3 ms |  7.4 ms |   3.4 ms |
| Retained JS heap             |                1.4 MB |  881 KB |   812 KB |
| Bundle, minified             |               27.1 KB | 25.4 KB |  36.5 KB |
| Bundle, gzip                 |                8.5 KB | 10.4 KB |  14.0 KB |

"Per call" repeats the calls 20000 times, so caches are warm. "Each call once" is the first render: no cache hits.
Retained heap is measured after the warm run and includes caches and the merge table.

The orisu bundle includes the merge table the compiler generates for this project: 1927 class names, 31 KB minified
and 11 KB gzip. It grows with the number of classes the project uses. The `orisu/cn` code itself is 2.6 KB gzip.
