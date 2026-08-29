#!/usr/bin/env python3
"""Benchmark harness for tokenloom — measures real wall-clock timings.

Usage:
    python3 tools/bench.py [--bin PATH] [--runs N]

Measures:
  - process startup (`--version`)
  - single-engine search
  - federated search (default category set, parallel)
  - page fetch, uncached
  - page fetch, cache hit

Search/fetch timings come from the binary's own `elapsed_ms` field (JSON
output), so they measure the operation only, not process spawn. Startup is
measured as full process wall clock. Results print as a Markdown table.
"""

from __future__ import annotations

import argparse
import json
import os
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

QUERY = "zig programming language"


def run(bin_path: str, args: list[str], env: dict | None = None) -> tuple[float, str]:
    start = time.perf_counter()
    proc = subprocess.run(
        [bin_path, *args],
        capture_output=True,
        text=True,
        timeout=60,
        env={**os.environ, **(env or {})},
    )
    wall = (time.perf_counter() - start) * 1000
    if proc.returncode != 0:
        raise RuntimeError(f"command failed ({proc.returncode}): {proc.stderr[:200]}")
    return wall, proc.stdout


def json_field(output: str, key: str):
    return json.loads(output).get(key)


def median(values: list[float]) -> float:
    return round(statistics.median(values), 1)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin", default="target/release/tokenloom")
    parser.add_argument("--runs", type=int, default=5)
    args = parser.parse_args()
    bin_path = args.bin
    runs = args.runs

    rows: list[tuple[str, str]] = []

    # ── process startup ────────────────────────────────────────────────────
    samples = []
    for _ in range(max(runs, 10)):
        wall, _ = run(bin_path, ["--version"])
        samples.append(wall)
    rows.append(("startup (`--version`, full process)", f"{median(samples)} ms (min {min(samples):.1f} ms)"))

    # ── single-engine search ───────────────────────────────────────────────
    samples, engines = [], "?"
    for _ in range(runs):
        _, out = run(bin_path, ["search", QUERY, "--engines", "wikipedia", "--json"])
        samples.append(json_field(out, "elapsed_ms"))
        engines = len(json_field(out, "engines_queried") or [])
    rows.append((f"search — single engine ({engines})", f"{median(samples)} ms (median of {runs})"))

    # ── federated search (default set, parallel) ───────────────────────────
    samples, engines = [], "?"
    for _ in range(runs):
        _, out = run(bin_path, ["search", QUERY, "--json"])
        body = json.loads(out)
        samples.append(body["elapsed_ms"])
        engines = len(body["engines_queried"] or [])
    rows.append((f"search — federated, {engines} engines in parallel", f"{median(samples)} ms (median of {runs})"))

    # ── all-implemented-engines fan-out ────────────────────────────────────
    # Enumerate every implemented engine, then fan out to all of them at once
    # with a raised cap and generous timeout.
    _, list_out = run(bin_path, ["engines", "list", "--implemented-only", "--json"])
    listing = json.loads(list_out)
    samples, engines, failed = [], 0, 0
    for _ in range(runs):
        _, out = run(
            bin_path,
            ["search", QUERY, "--engines", ",".join(e["name"] for e in listing), "--json", "--max-engines", "999"],
            env={"TOKENLOOM_TIMEOUT_MS": "15000"},
        )
        body = json.loads(out)
        samples.append(body["elapsed_ms"])
        engines = len(body["engines_queried"] or [])
        failed = len(body["engines_failed"] or [])
    rows.append(
        (
            f"search — every implemented engine in parallel ({engines})",
            f"{median(samples)} ms (median of {runs}; {failed} engine(s) failed/timed out)",
        )
    )

    # ── page fetch: uncached vs cache hit ──────────────────────────────────────
    url = "https://example.com"
    with tempfile.TemporaryDirectory() as tmp:
        cache_db = str(Path(tmp) / "cache.db")

        # cold: fresh cache DB, three sequential fetches of distinct paths
        cold = []
        for i in range(3):
            _, out = run(
                bin_path,
                ["fetch", f"https://example.com/r{i}", "--json"],
                env={"TOKENLOOM_CACHE_DB": cache_db},
            )
            cold.append(json_field(out, "elapsed_ms"))
        rows.append(("fetch — uncached (network + sanitiser)", f"{median(cold)} ms (median of 3)"))

        # warm: seed once, then measure cache hits
        run(bin_path, ["fetch", url, "--json"], env={"TOKENLOOM_CACHE_DB": cache_db})
        warm = []
        for _ in range(runs):
            _, out = run(
                bin_path,
                ["fetch", url, "--json"],
                env={"TOKENLOOM_CACHE_DB": cache_db},
            )
            warm.append(json_field(out, "elapsed_ms"))
        rows.append(("fetch — cache hit (SQLite, warm)", f"{median(warm)} ms (median of {runs})"))

    # ── binary size ────────────────────────────────────────────────────────
    size = Path(bin_path).stat().st_size / (1024 * 1024)
    rows.append(("release binary size", f"{size:.1f} MiB"))

    print("| Scenario | Result |")
    print("|---|---|")
    for label, value in rows:
        print(f"| {label} | {value} |")


if __name__ == "__main__":
    main()
