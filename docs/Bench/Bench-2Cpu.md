# Benchmark Comparison

Generated: 2026-01-24T18:18:18.218766

## Summary

| Mode | Connections | Requests/sec | MB/sec | p50 (μs) | p99 (μs) | Errors |
|------|-------------|--------------|--------|----------|----------|--------|
| dpdk | 10 | 82745 | 91.5 | 114 | 213 | 0 |
| dpdk | 50 | 217384 | 240.5 | 229 | 343 | 0 |
| dpdk | 100 | 217308 | 240.4 | 458 | 667 | 0 |
| dpdk | 200 | 224914 | 249.0 | 911 | 1310 | 0 |
| dpdk | 500 | 217973 | 241.3 | 1638 | 9783 | 0 |
| tokio | 10 | 68373 | 75.6 | 140 | 306 | 0 |
| tokio | 50 | 172421 | 190.7 | 281 | 465 | 0 |
| tokio | 100 | 183371 | 202.9 | 545 | 802 | 0 |
| tokio | 200 | 183862 | 203.5 | 1111 | 1614 | 0 |
| tokio | 500 | 180745 | 200.1 | 1929 | 11055 | 0 |
| tokio-local | 10 | 74018 | 81.8 | 126 | 279 | 0 |
| tokio-local | 50 | 177762 | 196.7 | 274 | 430 | 0 |
| tokio-local | 100 | 184628 | 204.2 | 537 | 699 | 0 |
| tokio-local | 200 | 180254 | 199.5 | 1098 | 1349 | 0 |
| tokio-local | 500 | 174338 | 193.0 | 2847 | 3301 | 0 |

## Throughput Comparison

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900, #33cc33"
---
xychart-beta
    title "Requests per Second by Connection Count"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "Requests/sec" 0 --> 247405
    line "dpdk" [82745, 217383, 217307, 224914, 217972]
    line "tokio" [68372, 172420, 183370, 183861, 180745]
    line "tokio-local" [74018, 177762, 184627, 180253, 174338]
```

**Legend:** dpdk (blue) | tokio (orange) | tokio-local (green)


## Bandwidth Comparison

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900, #33cc33"
---
xychart-beta
    title "MB per Second by Connection Count"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "MB/sec" 0 --> 273
    line "dpdk" [91, 240, 240, 249, 241]
    line "tokio" [75, 190, 202, 203, 200]
    line "tokio-local" [81, 196, 204, 199, 193]
```

**Legend:** dpdk (blue) | tokio (orange) | tokio-local (green)


## DPDK Throughput Improvement

Percentage improvement of DPDK over other modes (positive = DPDK is faster).

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900"
---
xychart-beta
    title "DPDK Throughput Improvement (%)"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "Improvement (%)" 1 --> 36
    line "vs tokio" [21, 26, 18, 22, 20]
    line "vs tokio-local" [11, 22, 17, 24, 25]
```

**Legend:** vs tokio (blue) | vs tokio-local (orange)


## DPDK Bandwidth Improvement

Percentage improvement of DPDK over other modes (positive = DPDK is faster).

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900"
---
xychart-beta
    title "DPDK Bandwidth Improvement (%)"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "Improvement (%)" 1 --> 36
    line "vs tokio" [21, 26, 18, 22, 20]
    line "vs tokio-local" [11, 22, 17, 24, 25]
```

**Legend:** vs tokio (blue) | vs tokio-local (orange)


## Latency Comparison (p50)

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900, #33cc33"
---
xychart-beta
    title "p50 Latency by Connection Count"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "Latency (μs)" 0 --> 3416
    line "dpdk" [114, 229, 458, 911, 1638]
    line "tokio" [140, 281, 545, 1111, 1929]
    line "tokio-local" [126, 274, 537, 1098, 2847]
```

**Legend:** dpdk (blue) | tokio (orange) | tokio-local (green)


## Latency Comparison (p99)

```mermaid
---
config:
    themeVariables:
        xyChart:
            plotColorPalette: "#3366cc, #ff9900, #33cc33"
---
xychart-beta
    title "p99 Latency by Connection Count"
    x-axis "Connections" [10, 50, 100, 200, 500]
    y-axis "Latency (μs)" 0 --> 13266
    line "dpdk" [213, 343, 667, 1310, 9783]
    line "tokio" [306, 465, 802, 1614, 11055]
    line "tokio-local" [279, 430, 699, 1349, 3301]
```

**Legend:** dpdk (blue) | tokio (orange) | tokio-local (green)


## Raw Data

### dpdk

<details>
<summary>Click to expand</summary>

```json
{
  "benchmark_time": "2026-01-25T02:16:29+00:00",
  "results": [
    {
      "url": "http://10.0.0.5:8080/",
      "connections": 10,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 1654902,
      "errors": 0,
      "gb_read": 1.79,
      "requests_per_sec": 82745.1,
      "mb_per_sec": 91.48,
      "latency": {
        "p50_us": 114,
        "p75_us": 137,
        "p90_us": 162,
        "p99_us": 213,
        "avg_us": 119,
        "max_us": 35167,
        "stdev_us": 65
      }
    },
    {
      "url": "http://10.0.0.5:8080/",
      "connections": 50,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 4347671,
      "errors": 0,
      "gb_read": 4.7,
      "requests_per_sec": 217383.55,
      "mb_per_sec": 240.48,
      "latency": {
        "p50_us": 229,
        "p75_us": 260,
        "p90_us": 288,
        "p99_us": 343,
        "avg_us": 228,
        "max_us": 9663,
        "stdev_us": 57
      }
    },
    {
      "url": "http://10.0.0.5:8080/",
      "connections": 100,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 4346151,
      "errors": 0,
      "gb_read": 4.7,
      "requests_per_sec": 217307.55,
      "mb_per_sec": 240.42,
      "latency": {
        "p50_us": 458,
        "p75_us": 527,
        "p90_us": 578,
        "p99_us": 667,
        "avg_us": 459,
        "max_us": 161407,
        "stdev_us": 551
      }
    },
    {
      "url": "http://10.0.0.5:8080/",
      "connections": 200,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 4498286,
      "errors": 0,
      "gb_read": 4.86,
      "requests_per_sec": 224914.3,
      "mb_per_sec": 249.03,
      "latency": {
        "p50_us": 911,
        "p75_us": 1041,
        "p90_us": 1147,
        "p99_us": 1310,
        "avg_us": 887,
        "max_us": 203775,
        "stdev_us": 268
      }
    },
    {
      "url": "http://10.0.0.5:8080/",
      "connections": 500,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 4359454,
      "errors": 0,
      "gb_read": 4.71,
      "requests_per_sec": 217972.7,
      "mb_per_sec": 241.34,
      "latency": {
        "p50_us": 1638,
        "p75_us": 2157,
        "p90_us": 4883,
        "p99_us": 9783,
        "avg_us": 2289,
        "max_us": 25007,
        "stdev_us": 1906
      }
    }
  ]
}
```

</details>

### tokio

<details>
<summary>Click to expand</summary>

```json
{
  "benchmark_time": "2026-01-24T22:08:21+00:00",
  "results": [
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 10,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 1367452,
      "errors": 0,
      "gb_read": 1.48,
      "requests_per_sec": 68372.6,
      "mb_per_sec": 75.59,
      "latency": {
        "p50_us": 140,
        "p75_us": 177,
        "p90_us": 231,
        "p99_us": 306,
        "avg_us": 149,
        "max_us": 2413,
        "stdev_us": 55
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 50,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3448417,
      "errors": 0,
      "gb_read": 3.73,
      "requests_per_sec": 172420.85,
      "mb_per_sec": 190.74,
      "latency": {
        "p50_us": 281,
        "p75_us": 330,
        "p90_us": 388,
        "p99_us": 465,
        "avg_us": 289,
        "max_us": 7827,
        "stdev_us": 79
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 100,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3667411,
      "errors": 0,
      "gb_read": 3.96,
      "requests_per_sec": 183370.55,
      "mb_per_sec": 202.86,
      "latency": {
        "p50_us": 545,
        "p75_us": 626,
        "p90_us": 687,
        "p99_us": 802,
        "avg_us": 544,
        "max_us": 207743,
        "stdev_us": 184
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 200,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3677237,
      "errors": 0,
      "gb_read": 3.97,
      "requests_per_sec": 183861.85,
      "mb_per_sec": 203.5,
      "latency": {
        "p50_us": 1111,
        "p75_us": 1271,
        "p90_us": 1395,
        "p99_us": 1614,
        "avg_us": 1085,
        "max_us": 7431,
        "stdev_us": 269
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 500,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3614900,
      "errors": 0,
      "gb_read": 3.91,
      "requests_per_sec": 180745.0,
      "mb_per_sec": 200.12,
      "latency": {
        "p50_us": 1929,
        "p75_us": 2689,
        "p90_us": 5967,
        "p99_us": 11055,
        "avg_us": 2761,
        "max_us": 29711,
        "stdev_us": 2215
      }
    }
  ]
}
```

</details>

### tokio-local

<details>
<summary>Click to expand</summary>

```json
{
  "benchmark_time": "2026-01-24T22:10:53+00:00",
  "results": [
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 10,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 1480363,
      "errors": 0,
      "gb_read": 1.6,
      "requests_per_sec": 74018.15,
      "mb_per_sec": 81.83,
      "latency": {
        "p50_us": 126,
        "p75_us": 155,
        "p90_us": 193,
        "p99_us": 279,
        "avg_us": 132,
        "max_us": 2445,
        "stdev_us": 47
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 50,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3555250,
      "errors": 0,
      "gb_read": 3.84,
      "requests_per_sec": 177762.5,
      "mb_per_sec": 196.65,
      "latency": {
        "p50_us": 274,
        "p75_us": 303,
        "p90_us": 355,
        "p99_us": 430,
        "avg_us": 281,
        "max_us": 207871,
        "stdev_us": 153
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 100,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3692558,
      "errors": 0,
      "gb_read": 3.99,
      "requests_per_sec": 184627.9,
      "mb_per_sec": 204.25,
      "latency": {
        "p50_us": 537,
        "p75_us": 568,
        "p90_us": 613,
        "p99_us": 699,
        "avg_us": 539,
        "max_us": 206975,
        "stdev_us": 168
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 200,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3605078,
      "errors": 0,
      "gb_read": 3.9,
      "requests_per_sec": 180253.9,
      "mb_per_sec": 199.52,
      "latency": {
        "p50_us": 1098,
        "p75_us": 1188,
        "p90_us": 1231,
        "p99_us": 1349,
        "avg_us": 1106,
        "max_us": 10783,
        "stdev_us": 119
      }
    },
    {
      "url": "http://10.0.0.4:8080/",
      "connections": 500,
      "duration_secs": 20.0,
      "mode": "raw",
      "worker_threads": 2,
      "timeout_ms": 5000,
      "requests": 3486766,
      "errors": 0,
      "gb_read": 3.77,
      "requests_per_sec": 174338.3,
      "mb_per_sec": 193.03,
      "latency": {
        "p50_us": 2847,
        "p75_us": 2953,
        "p90_us": 3045,
        "p99_us": 3301,
        "avg_us": 2865,
        "max_us": 24847,
        "stdev_us": 257
      }
    }
  ]
}
```

</details>
