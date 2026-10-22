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
| compile                |                 253.2M | 29.2M | 8.7× |         9.0 ms |   2.1 ms |         106.3 MB |  69.2 MB |
| scan                   |                  4.86G | 4.87G | 1.0× |       140.5 ms | 106.1 ms |         143.9 MB | 122.8 MB |
| scan, cached           |                        | 997.5M |     |                |  26.3 ms |                  | 148.6 MB |
| build                  |                  1.67G | 166.4M | 10.1× |       62.9 ms |  12.2 ms |         232.1 MB | 155.7 MB |
| cold start             |                  7.11G | 5.05G | 1.4× |       222.5 ms | 121.6 ms |         232.2 MB | 138.3 MB |
| HMR                    |                 287.2M | 18.4M | 15.6× |       14.4 ms |   1.1 ms |         298.2 MB | 122.6 MB |
| HMR, no new class      |                   4.8M |  3.8M | 1.2× |         142 µs |   151 µs |         170.2 MB | 110.1 MB |
| canonicalize, cold     |                 34.57G | 1.17G | 29.6× |    1560.7 ms |  30.2 ms |         879.1 MB | 106.5 MB |
| canonicalize, warm     |                 221.0M |  1.3M | 176.1× |       10.0 ms |    69 µs |         422.7 MB |  86.0 MB |

Tailwind has no scan cache, so `scan, cached` has no counterpart.

## cn

The 1524 `cn(…)` calls (727 distinct) found in the project, against `clsx` 2.1.1 + `tailwind-merge` 3.7.0 (the classic
shadcn/ui `cn`) and [`cn`](https://github.com/shadcn-ui/cn) 0.2.6.

|                              | clsx + tailwind-merge |      cn | orisu/cn |
| ---------------------------- | --------------------: | ------: | -------: |
| Instructions per call        |                 35.0k |   13.8k |    10.3k |
| Time per call                |               1.51 µs | 0.52 µs |  0.29 µs |
| Instructions, each call once |                137.2M |  222.4M |   107.4M |
| Time, each call once         |                5.2 ms |  7.4 ms |   3.5 ms |
| Retained JS heap             |                1.4 MB |  890 KB |   949 KB |
| Bundle, minified             |               27.1 KB | 25.4 KB | 114.1 KB |
| Bundle, gzip                 |                8.5 KB | 10.4 KB |  17.5 KB |

"Per call" repeats the calls 20000 times, so caches are warm. "Each call once" is the first render: no cache hits.
Retained heap is measured after the warm run and includes caches and the merge table.

The orisu bundle includes the merge table the compiler generates for this project: 1927 class shapes, 109 KB minified
and 15 KB gzip. It grows with the number of classes the project uses. The `orisu/cn` code itself is under 4.5 KB gzip.
