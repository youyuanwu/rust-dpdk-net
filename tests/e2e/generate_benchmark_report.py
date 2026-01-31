#!/usr/bin/env python3
"""
Generate benchmark comparison Markdown with Mermaid charts.

Reads summary.json from each mode directory and creates a comparison report.
Modes are auto-detected from subdirectories containing summary.json.
"""

import json
import os
from pathlib import Path
from datetime import datetime

BENCHMARKS_DIR = Path(__file__).parent.parent.parent / "build" / "benchmarks"
OUTPUT_FILE = BENCHMARKS_DIR / "BENCHMARK_COMPARISON.md"

# Preferred order for display (modes not in this list appear at the end alphabetically)
MODE_ORDER = ["dpdk", "tokio", "tokio-local", "kimojio", "kimojio-poll"]

# Color palette for charts
CHART_COLORS = ["#3366cc", "#ff9900", "#33cc33", "#9933ff", "#cc3366", "#33cccc", "#cc6633"]
COLOR_NAMES = ["blue", "orange", "green", "purple", "pink", "cyan", "brown"]


def discover_modes() -> list[str]:
    """Discover available modes by scanning for directories with summary.json."""
    if not BENCHMARKS_DIR.exists():
        return []
    
    modes = []
    for entry in BENCHMARKS_DIR.iterdir():
        if entry.is_dir() and (entry / "summary.json").exists():
            modes.append(entry.name)
    
    # Sort: known modes first in preferred order, then unknown modes alphabetically
    def sort_key(mode):
        if mode in MODE_ORDER:
            return (0, MODE_ORDER.index(mode))
        return (1, mode)
    
    return sorted(modes, key=sort_key)


def load_summary(mode: str) -> dict | None:
    """Load summary.json for a given mode."""
    summary_path = BENCHMARKS_DIR / mode / "summary.json"
    if not summary_path.exists():
        return None
    with open(summary_path) as f:
        return json.load(f)


def get_results_by_connections(summary: dict) -> dict:
    """Index results by connection count."""
    return {r["connections"]: r for r in summary["results"]}


def get_chart_colors(num_modes: int) -> str:
    """Get comma-separated color palette for the given number of modes."""
    return ", ".join(CHART_COLORS[:num_modes])


def add_legend(modes: list[str]) -> list[str]:
    """Generate legend lines for the given modes."""
    legend_items = [f"{m} ({COLOR_NAMES[i % len(COLOR_NAMES)]})" for i, m in enumerate(modes)]
    return ["", "**Legend:** " + " | ".join(legend_items), ""]


def generate_markdown() -> str:
    """Generate the comparison Markdown content."""
    # Discover and load all summaries
    modes = discover_modes()
    if not modes:
        return "# Benchmark Comparison\n\nNo benchmark data found.\n"
    
    summaries = {}
    for mode in modes:
        summary = load_summary(mode)
        if summary:
            summaries[mode] = get_results_by_connections(summary)

    if not summaries:
        return "# Benchmark Comparison\n\nNo benchmark data found.\n"

    # Get all connection counts (sorted)
    all_connections = sorted(
        set(c for results in summaries.values() for c in results.keys())
    )

    chart_colors = get_chart_colors(len(modes))

    # Build markdown
    lines = [
        "# Benchmark Comparison",
        "",
        f"Generated: {datetime.now().isoformat()}",
        "",
        f"Modes tested: {', '.join(modes)}",
        "",
        "## Summary",
        "",
        "| Mode | Connections | Requests/sec | MB/sec | p50 (μs) | p99 (μs) | Errors |",
        "|------|-------------|--------------|--------|----------|----------|--------|",
    ]

    for mode in modes:
        if mode not in summaries:
            continue
        for conn in all_connections:
            if conn not in summaries[mode]:
                continue
            r = summaries[mode][conn]
            lat = r.get("latency", {})
            lines.append(
                f"| {mode} | {conn} | {r['requests_per_sec']:.0f} | {r['mb_per_sec']:.1f} | "
                f"{lat.get('p50_us', 'N/A')} | {lat.get('p99_us', 'N/A')} | {r['errors']} |"
            )

    # Throughput chart
    lines.extend([
        "",
        "## Throughput Comparison",
        "",
        "```mermaid",
        "---",
        "config:",
        "    themeVariables:",
        "        xyChart:",
        f'            plotColorPalette: "{chart_colors}"',
        "---",
        "xychart-beta",
        '    title "Requests per Second by Connection Count"',
        f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
    ])

    # Find max for y-axis
    max_rps = max(
        r["requests_per_sec"]
        for results in summaries.values()
        for r in results.values()
    )
    y_max = int(max_rps * 1.1)
    lines.append(f'    y-axis "Requests/sec" 0 --> {y_max}')

    for mode in modes:
        if mode not in summaries:
            continue
        values = [
            str(int(summaries[mode].get(c, {}).get("requests_per_sec", 0)))
            for c in all_connections
        ]
        lines.append(f'    line "{mode}" [{", ".join(values)}]')

    lines.append("```")
    lines.extend(add_legend(modes))

    # Bandwidth chart (MB/sec)
    lines.extend([
        "",
        "## Bandwidth Comparison",
        "",
        "```mermaid",
        "---",
        "config:",
        "    themeVariables:",
        "        xyChart:",
        f'            plotColorPalette: "{chart_colors}"',
        "---",
        "xychart-beta",
        '    title "MB per Second by Connection Count"',
        f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
    ])

    max_mbps = max(
        r["mb_per_sec"]
        for results in summaries.values()
        for r in results.values()
    )
    y_max_mbps = int(max_mbps * 1.1)
    lines.append(f'    y-axis "MB/sec" 0 --> {y_max_mbps}')

    for mode in modes:
        if mode not in summaries:
            continue
        values = [
            str(int(summaries[mode].get(c, {}).get("mb_per_sec", 0)))
            for c in all_connections
        ]
        lines.append(f'    line "{mode}" [{", ".join(values)}]')

    lines.append("```")
    lines.extend(add_legend(modes))

    # DPDK improvement percentage helper
    def calc_improvement(dpdk_val, other_val):
        """Calculate percentage improvement of DPDK over other mode."""
        if other_val == 0:
            return 0
        return ((dpdk_val - other_val) / other_val) * 100

    # Throughput improvement chart (DPDK vs others)
    if "dpdk" in summaries:
        other_modes = [m for m in modes if m != "dpdk" and m in summaries]
        if other_modes:
            improvement_colors = get_chart_colors(len(other_modes))
            lines.extend([
                "",
                "## DPDK Throughput Improvement",
                "",
                "Percentage improvement of DPDK over other modes (positive = DPDK is faster).",
                "",
                "```mermaid",
                "---",
                "config:",
                "    themeVariables:",
                "        xyChart:",
                f'            plotColorPalette: "{improvement_colors}"',
                "---",
                "xychart-beta",
                '    title "DPDK Throughput Improvement (%)"',
                f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
            ])

            # Calculate min/max for y-axis
            all_improvements = []
            for other_mode in other_modes:
                for c in all_connections:
                    dpdk_val = summaries["dpdk"].get(c, {}).get("requests_per_sec", 0)
                    other_val = summaries[other_mode].get(c, {}).get("requests_per_sec", 0)
                    all_improvements.append(calc_improvement(dpdk_val, other_val))

            y_min = int(min(all_improvements) - 10)
            y_max = int(max(all_improvements) + 10)
            lines.append(f'    y-axis "Improvement (%)" {y_min} --> {y_max}')

            for other_mode in other_modes:
                values = []
                for c in all_connections:
                    dpdk_val = summaries["dpdk"].get(c, {}).get("requests_per_sec", 0)
                    other_val = summaries[other_mode].get(c, {}).get("requests_per_sec", 0)
                    improvement = calc_improvement(dpdk_val, other_val)
                    values.append(str(int(improvement)))
                lines.append(f'    line "vs {other_mode}" [{", ".join(values)}]')

            lines.append("```")
            # Dynamic legend for improvement chart
            improvement_legend_items = [
                f"vs {m} ({COLOR_NAMES[i % len(COLOR_NAMES)]})"
                for i, m in enumerate(other_modes)
            ]
            lines.extend([
                "",
                "**Legend:** " + " | ".join(improvement_legend_items),
                "",
            ])

    # Latency p50 chart
    lines.extend([
        "",
        "## Latency Comparison (p50)",
        "",
        "```mermaid",
        "---",
        "config:",
        "    themeVariables:",
        "        xyChart:",
        f'            plotColorPalette: "{chart_colors}"',
        "---",
        "xychart-beta",
        '    title "p50 Latency by Connection Count"',
        f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
    ])

    max_p50 = max(
        r.get("latency", {}).get("p50_us", 0)
        for results in summaries.values()
        for r in results.values()
    )
    y_max_lat = int(max_p50 * 1.2)
    lines.append(f'    y-axis "Latency (μs)" 0 --> {y_max_lat}')

    for mode in modes:
        if mode not in summaries:
            continue
        values = [
            str(summaries[mode].get(c, {}).get("latency", {}).get("p50_us", 0))
            for c in all_connections
        ]
        lines.append(f'    line "{mode}" [{", ".join(values)}]')

    lines.append("```")
    lines.extend(add_legend(modes))

    # Latency p90 chart
    lines.extend([
        "",
        "## Latency Comparison (p90)",
        "",
        "```mermaid",
        "---",
        "config:",
        "    themeVariables:",
        "        xyChart:",
        f'            plotColorPalette: "{chart_colors}"',
        "---",
        "xychart-beta",
        '    title "p90 Latency by Connection Count"',
        f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
    ])

    max_p90 = max(
        r.get("latency", {}).get("p90_us", 0)
        for results in summaries.values()
        for r in results.values()
    )
    y_max_p90 = int(max_p90 * 1.2)
    lines.append(f'    y-axis "Latency (μs)" 0 --> {y_max_p90}')

    for mode in modes:
        if mode not in summaries:
            continue
        values = [
            str(summaries[mode].get(c, {}).get("latency", {}).get("p90_us", 0))
            for c in all_connections
        ]
        lines.append(f'    line "{mode}" [{", ".join(values)}]')

    lines.append("```")
    lines.extend(add_legend(modes))

    # Latency p99 chart
    lines.extend([
        "",
        "## Latency Comparison (p99)",
        "",
        "```mermaid",
        "---",
        "config:",
        "    themeVariables:",
        "        xyChart:",
        f'            plotColorPalette: "{chart_colors}"',
        "---",
        "xychart-beta",
        '    title "p99 Latency by Connection Count"',
        f'    x-axis "Connections" [{", ".join(str(c) for c in all_connections)}]',
    ])

    max_p99 = max(
        r.get("latency", {}).get("p99_us", 0)
        for results in summaries.values()
        for r in results.values()
    )
    y_max_p99 = int(max_p99 * 1.2)
    lines.append(f'    y-axis "Latency (μs)" 0 --> {y_max_p99}')

    for mode in modes:
        if mode not in summaries:
            continue
        values = [
            str(summaries[mode].get(c, {}).get("latency", {}).get("p99_us", 0))
            for c in all_connections
        ]
        lines.append(f'    line "{mode}" [{", ".join(values)}]')

    lines.append("```")
    lines.extend(add_legend(modes))

    # Raw data section
    lines.extend([
        "",
        "## Raw Data",
        "",
    ])

    for mode in modes:
        if mode not in summaries:
            continue
        lines.append(f"### {mode}")
        lines.append("")
        lines.append("<details>")
        lines.append("<summary>Click to expand</summary>")
        lines.append("")
        lines.append("```json")
        summary = load_summary(mode)
        lines.append(json.dumps(summary, indent=2))
        lines.append("```")
        lines.append("")
        lines.append("</details>")
        lines.append("")

    return "\n".join(lines)


def main():
    """Main entry point."""
    if not BENCHMARKS_DIR.exists():
        print(f"Error: Benchmarks directory not found: {BENCHMARKS_DIR}")
        return 1

    content = generate_markdown()

    with open(OUTPUT_FILE, "w") as f:
        f.write(content)

    print(f"Generated: {OUTPUT_FILE}")
    return 0


if __name__ == "__main__":
    exit(main())
