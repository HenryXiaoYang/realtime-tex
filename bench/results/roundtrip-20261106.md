# Round-trip benchmark

Machine: Intel(R) Xeon(R) Processor @ 2.10GHz (4 threads)  
Hardware factor h = 1.04 (in-engine line breaking vs the paper's Table 1)

## In-engine line breaking (upstream `texlode/luatex-benchmark`, vendored in bench/upstream)

| Category | Paper | Here (amortized median) |
|---|---|---|
| short | 0.17 ms | 0.041 ms |
| medium | 0.70 ms | 0.727 ms |
| long | 1.99 ms | 1.692 ms |
| inline-math | 0.20 ms | 0.164 ms |
| display-math | 0.11 ms | 0.095 ms |

## Warm-session round trips (apply_edit → ParagraphUpdate in hand)

| Project | Category | Lines | Glyphs | Individual median | P95 | TeX | Traverse | Pack | Amortized median | Overhead share |
|---|---|---|---|---|---|---|---|---|---|---|
| book-10-pure-lmtfm | short | 1 | 35 | 0.289 ms | 0.467 | 0.094 | 0.044 | 0.000 | 0.154 ms | 67 % |
| book-10-pure-lmtfm | medium | 4 | 215 | 0.664 ms | 1.030 | 0.312 | 0.163 | 0.000 | 0.476 ms | 53 % |
| book-10-pure-lmtfm | inline-math | 3 | 170 | 0.597 ms | 0.956 | 0.240 | 0.150 | 0.000 | 0.394 ms | 60 % |
| book-100-pure-lmtfm | short | 1 | 22 | 0.318 ms | 0.532 | 0.096 | 0.040 | 0.000 | 0.155 ms | 70 % |
| book-100-pure-lmtfm | medium | 4 | 263 | 0.930 ms | 1.892 | 0.440 | 0.216 | 0.000 | 0.668 ms | 53 % |
| book-100-pure-lmtfm | long | 10 | 625 | 1.649 ms | 2.204 | 0.925 | 0.413 | 0.000 | 1.330 ms | 44 % |
| book-100-pure-lmtfm | inline-math | 3 | 147 | 0.631 ms | 0.875 | 0.251 | 0.142 | 0.000 | 0.431 ms | 60 % |
| book-300-pure-lmtfm | short | 1 | 34 | 0.371 ms | 0.558 | 0.111 | 0.048 | 0.000 | 0.244 ms | 70 % |
| book-300-pure-lmtfm | medium | 4 | 229 | 0.880 ms | 1.460 | 0.436 | 0.193 | 0.000 | 0.730 ms | 50 % |
| book-300-pure-lmtfm | long | 11 | 639 | 1.554 ms | 2.924 | 0.770 | 0.441 | 0.000 | 1.317 ms | 50 % |
| book-300-pure-lmtfm | inline-math | 3 | 139 | 0.766 ms | 1.079 | 0.363 | 0.159 | 0.000 | 0.561 ms | 53 % |
| book-10-units-lmtfm | short | 1 | 32 | 0.373 ms | 0.604 | 0.125 | 0.047 | 0.000 | 0.155 ms | 66 % |
| book-10-units-lmtfm | inline-math | 3 | 154 | 0.695 ms | 1.035 | 0.273 | 0.173 | 0.000 | 0.416 ms | 61 % |
| book-10-units-lmtfm | display-math | 5 | 236 | 0.939 ms | 1.243 | 0.499 | 0.203 | 0.000 | 0.811 ms | 47 % |
| book-10-units-lmtfm | footnote | 5 | 280 | 1.551 ms | 1.844 | 0.958 | 0.243 | 0.000 | 1.306 ms | 38 % |
| book-10-units-lmtfm | list | 3 | 96 | 1.138 ms | 1.552 | 0.747 | 0.151 | 0.000 | 0.941 ms | 34 % |
| book-10-units-lmtfm | figure | 3 | 103 | 1.465 ms | 2.093 | 0.990 | 0.135 | 0.000 | 1.125 ms | 32 % |
| book-10-units-lmtfm | table | 2 | 71 | 1.137 ms | 1.541 | 0.699 | 0.139 | 0.000 | 0.949 ms | 38 % |
| book-10-units-lmtfm | heading | 1 | 13 | 0.574 ms | 0.854 | 0.312 | 0.045 | 0.000 | 0.354 ms | 46 % |
| book-100-units-lmtfm | short | 1 | 38 | 0.431 ms | 0.591 | 0.130 | 0.054 | 0.000 | 0.179 ms | 70 % |
| book-100-units-lmtfm | medium | 5 | 263 | 0.809 ms | 1.065 | 0.397 | 0.195 | 0.000 | 0.601 ms | 51 % |
| book-100-units-lmtfm | long | 12 | 735 | 1.581 ms | 1.911 | 0.862 | 0.432 | 0.000 | 1.444 ms | 45 % |
| book-100-units-lmtfm | inline-math | 2 | 117 | 0.697 ms | 0.925 | 0.339 | 0.123 | 0.000 | 0.422 ms | 51 % |
| book-100-units-lmtfm | display-math | 5 | 195 | 1.082 ms | 1.499 | 0.592 | 0.218 | 0.000 | 0.787 ms | 45 % |
| book-100-units-lmtfm | footnote | 3 | 187 | 1.393 ms | 2.008 | 0.869 | 0.222 | 0.000 | 1.126 ms | 38 % |
| book-100-units-lmtfm | list | 3 | 97 | 1.129 ms | 1.482 | 0.735 | 0.151 | 0.000 | 0.980 ms | 35 % |
| book-100-units-lmtfm | figure | 3 | 105 | 1.868 ms | 3.090 | 1.105 | 0.165 | 0.000 | 1.156 ms | 41 % |
| book-100-units-lmtfm | table | 2 | 74 | 1.155 ms | 1.392 | 0.694 | 0.137 | 0.000 | 0.942 ms | 40 % |
| book-100-units-lmtfm | heading | 1 | 15 | 0.614 ms | 0.868 | 0.310 | 0.045 | 0.000 | 0.374 ms | 50 % |

Paper references: short round trip 0.79 ms, medium 6.11 ms.

## Gates

| Gate | Value | Limit | Pass | Note |
|---|---|---|---|---|
| size independence short (300p/10p individual median) | 1.285 | 1.250 | **no** | 0.371 ms vs 0.289 ms |
| size independence short (100p/10p individual median) | 1.103 | 1.250 | yes |  |
| size independence medium (300p/10p individual median) | 1.325 | 1.250 | **no** | 0.880 ms vs 0.664 ms |
| size independence medium (100p/10p individual median) | 1.401 | 1.250 | **no** |  |
| size independence inline-math (300p/10p individual median) | 1.282 | 1.250 | **no** | 0.766 ms vs 0.597 ms |
| size independence inline-math (100p/10p individual median) | 1.056 | 1.250 | yes |  |
| short amortized median ≤ 1.0 ms × h | 0.244 | 1.039 | yes | h = 1.04 |
| short individual median ≤ 1.5 ms × h | 0.371 | 1.558 | yes | P95 0.558 ms |
| short P95 ≤ 3 × median | 1.505 | 3.000 | yes |  |
| short IPC + host overhead ≤ 0.5 ms × h | 0.212 | 0.519 | yes | overhead share 70 % (paper: 18 %) |
| medium individual median ≤ 6.11 ms × h (paper reference) | 0.880 | 6.346 | yes |  |
| medium traversal + serialization ≤ 0.5 ms | 0.193 | 0.500 | yes |  |
| medium IPC + host overhead ≤ 0.5 ms × h | 0.251 | 0.519 | yes | overhead share 50 % (paper: 18 %) |
| display-math individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 1.082 | 5.193 | yes | P95 1.499 ms, tex 0.592 ms, 5 rows |
| size independence display-math (100p/10p individual median) | 1.152 | 1.250 | yes | 1.082 ms vs 0.939 ms |
| footnote individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 1.393 | 5.193 | yes | P95 2.008 ms, tex 0.869 ms, 3 rows |
| size independence footnote (100p/10p individual median) | 0.898 | 1.250 | yes | 1.393 ms vs 1.551 ms |
| list individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 1.129 | 5.193 | yes | P95 1.482 ms, tex 0.735 ms, 3 rows |
| size independence list (100p/10p individual median) | 0.992 | 1.250 | yes | 1.129 ms vs 1.138 ms |
| figure individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 1.868 | 5.193 | yes | P95 3.090 ms, tex 1.105 ms, 3 rows |
| size independence figure (100p/10p individual median) | 1.275 | 1.250 | **no** | 1.868 ms vs 1.465 ms |
| table individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 1.155 | 5.193 | yes | P95 1.392 ms, tex 0.694 ms, 2 rows |
| size independence table (100p/10p individual median) | 1.016 | 1.250 | yes | 1.155 ms vs 1.137 ms |
| heading individual median ≤ 5.0 ms × h (book-100-units-lmtfm) | 0.614 | 5.193 | yes | P95 0.868 ms, tex 0.310 ms, 1 rows |
| size independence heading (100p/10p individual median) | 1.070 | 1.250 | yes | 0.614 ms vs 0.574 ms |

Overall: some gates fail
