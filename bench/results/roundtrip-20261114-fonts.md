# Round-trip benchmark

Machine: Intel(R) Xeon(R) Processor @ 2.10GHz (4 threads)  
Hardware factor h = 1.19 (in-engine line breaking vs the paper's Table 1)

## In-engine line breaking (upstream `texlode/luatex-benchmark`, vendored in bench/upstream)

| Category | Paper | Here (amortized median) |
|---|---|---|
| short | 0.17 ms | 0.065 ms |
| medium | 0.70 ms | 0.831 ms |
| long | 1.99 ms | 2.245 ms |
| inline-math | 0.20 ms | 0.220 ms |
| display-math | 0.11 ms | 0.122 ms |

## Warm-session round trips (apply_edit → ParagraphUpdate in hand)

| Project | Category | Lines | Glyphs | Individual median | P95 | TeX | Traverse | Pack | Amortized median | Overhead share |
|---|---|---|---|---|---|---|---|---|---|---|
| book-300-pure-lmtfm | short | 1 | 34 | 0.550 ms | 0.679 | 0.164 | 0.073 | 0.000 | 0.209 ms | 70 % |
| book-300-pure-lmtfm | medium | 4 | 229 | 1.178 ms | 1.611 | 0.571 | 0.236 | 0.000 | 0.808 ms | 52 % |
| book-300-pure-lmtfm | long | 11 | 639 | 2.014 ms | 3.828 | 1.000 | 0.575 | 0.000 | 1.818 ms | 50 % |
| book-300-pure-lmtfm | inline-math | 3 | 139 | 1.005 ms | 1.581 | 0.448 | 0.184 | 0.000 | 0.740 ms | 55 % |
| book-300-pure-base | short | 1 | 34 | 0.566 ms | 0.718 | 0.167 | 0.079 | 0.000 | 0.228 ms | 70 % |
| book-300-pure-base | medium | 4 | 229 | 1.180 ms | 1.500 | 0.592 | 0.220 | 0.000 | 1.111 ms | 50 % |
| book-300-pure-base | long | 11 | 640 | 2.207 ms | 5.022 | 1.232 | 0.547 | 0.000 | 2.145 ms | 44 % |
| book-300-pure-base | inline-math | 3 | 139 | 1.086 ms | 1.343 | 0.499 | 0.191 | 0.000 | 0.889 ms | 54 % |
| book-300-pure | short | 1 | 34 | 0.805 ms | 0.977 | 0.415 | 0.077 | 0.000 | 0.448 ms | 48 % |
| book-300-pure | medium | 4 | 229 | 3.633 ms | 4.090 | 2.967 | 0.224 | 0.000 | 3.468 ms | 18 % |
| book-300-pure | long | 11 | 640 | 8.757 ms | 17.216 | 7.547 | 0.603 | 0.000 | 8.444 ms | 14 % |
| book-300-pure | inline-math | 3 | 139 | 2.930 ms | 3.475 | 2.253 | 0.211 | 0.000 | 2.147 ms | 23 % |

Paper references: short round trip 0.79 ms, medium 6.11 ms.

## Gates

| Gate | Value | Limit | Pass | Note |
|---|---|---|---|---|
| short amortized median ≤ 1.0 ms × h | 0.209 | 1.187 | yes | h = 1.19 |
| short individual median ≤ 1.5 ms × h | 0.550 | 1.781 | yes | P95 0.679 ms |
| short P95 ≤ 3 × median | 1.234 | 3.000 | yes |  |
| short IPC + host overhead ≤ 0.5 ms × h | 0.313 | 0.594 | yes | overhead share 70 % (paper: 18 %) |
| medium individual median ≤ 6.11 ms × h (paper reference) | 1.178 | 7.253 | yes |  |
| medium traversal + serialization ≤ 0.5 ms | 0.236 | 0.500 | yes |  |
| medium IPC + host overhead ≤ 0.5 ms × h | 0.371 | 0.594 | yes | overhead share 52 % (paper: 18 %) |

Overall: all gates pass
