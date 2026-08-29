#!/usr/bin/env python3
"""Generate engines.toml from PLAN.md Appendix A (SearXNG Configured Engines tables).

The generated file is committed to the repo; re-run after updating PLAN.md:

    python3 tools/gen_engines_toml.py

Validates: 248 unique engines, unique bangs, known categories, waves 1-3.
Engines listed under several tabs are emitted once with merged categories.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PLAN = ROOT / "PLAN.md"
OUT = ROOT / "engines.toml"

TAB_RE = re.compile(r"^### Tab `!([a-z_]+)` \(\d+ engines\)\s*$")
ROW_RE = re.compile(
    r"^\|\s*(?P<name>[^|]+?)\s*\|\s*`!(?P<bang>[^`]+)`\s*\|\s*`(?P<family>[^`]+)`\s*\|"
    r"\s*(?P<enabled>on|off)\s*\|\s*(?P<timeout>[\d.]+)\s*\|"
    r"\s*(?P<weight>[\d.]+)\s*\|"
    r"\s*(?P<paging>✓|—|n/a)\s*\|\s*(?P<locale>✓|—|n/a)\s*\|\s*"
    r"(?P<safe>✓|—|n/a)\s*\|\s*(?P<range>✓|—|n/a)\s*\|\s*(?P<wave>[123])\s*\|"
)
REGION_RE = re.compile(r"\s*\((?:NO|DE|CH|FR|ZH|KO|IT|JA|CZ)\)\s*$")

EXPECTED = {
    "general": 61, "images": 49, "videos": 32, "news": 16, "map": 3,
    "music": 11, "it": 44, "science": 10, "files": 17, "social_media": 9,
}

def engine_id(display: str) -> str:
    name = REGION_RE.sub("", display).strip().lower()
    return re.sub(r"\s+", "_", name)

def cap(v: str) -> bool:
    return v == "✓"

def toml_str(s: str) -> str:
    return '"' + s.replace("\\", "\\\\").replace('"', '\\"') + '"'

def main() -> int:
    lines = PLAN.read_text(encoding="utf-8").splitlines()
    current_tab = None
    order: list[str] = []
    engines: dict[str, dict] = {}
    bang_owner: dict[str, str] = {}
    counts: dict[str, int] = {}

    for raw in lines:
        m = TAB_RE.match(raw)
        if m:
            current_tab = m.group(1)
            counts.setdefault(current_tab, 0)
            continue
        if current_tab is None:
            continue
        m = ROW_RE.match(raw.strip())
        if not m:
            continue
        d = m.groupdict()
        eid = engine_id(d["name"])
        bang = d["bang"]
        if eid not in engines:
            order.append(eid)
            engines[eid] = {
                "name": eid,
                "display": d["name"].strip(),
                "bang": bang,
                "family": d["family"],
                "categories": [],
                "enabled": d["enabled"] == "on",
                "timeout_ms": round(float(d["timeout"]) * 1000),
                "weight": float(d["weight"]),
                "paging": cap(d["paging"]),
                "locale": cap(d["locale"]),
                "safe_search": cap(d["safe"]),
                "time_range": cap(d["range"]),
                "wave": int(d["wave"]),
            }
            if bang in bang_owner and bang_owner[bang] != eid:
                print(f"FATAL: bang !{bang} duplicated by {bang_owner[bang]} and {eid}", file=sys.stderr)
                return 1
            bang_owner[bang] = eid
        eng = engines[eid]
        if current_tab not in eng["categories"]:
            eng["categories"].append(current_tab)
        counts[current_tab] += 1

    for tab, expected in EXPECTED.items():
        got = counts.get(tab, 0)
        if got != expected:
            print(f"FATAL: tab {tab}: expected {expected} rows, parsed {got}", file=sys.stderr)
            return 1
    if len(order) != 248:
        print(f"FATAL: expected 248 unique engines, parsed {len(order)}", file=sys.stderr)
        return 1

    out = [
        "# engines.toml — tokenloom master engine registry (GENERATED FILE, do not edit by hand).",
        "# Source: PLAN.md Appendix A (SearXNG *Configured Engines*).",
        "# Regenerate with: python3 tools/gen_engines_toml.py",
        "#",
        "# 248 unique engines across 10 category tabs; each entry is one SearXNG engine",
        "# instance. Request/response extraction specs for declarative engines",
        "# (families `json_engine`, `xpath`, `css_engine`) are merged in from",
        "# tokenloom-engines built-in specs at build time.",
        "schema_version = 1",
        "",
        "engines = [",
    ]
    for eid in order:
        e = engines[eid]
        cats = ", ".join(toml_str(c) for c in e["categories"])
        fields = [
            f"name = {toml_str(e['name'])}",
            f"display = {toml_str(e['display'])}",
            f"bang = {toml_str(e['bang'])}",
            f"family = {toml_str(e['family'])}",
            f"categories = [{cats}]",
            f"enabled = {'true' if e['enabled'] else 'false'}",
            f"timeout_ms = {e['timeout_ms']}",
            f"weight = {e['weight']:g}",
            f"paging = {'true' if e['paging'] else 'false'}",
            f"locale = {'true' if e['locale'] else 'false'}",
            f"safe_search = {'true' if e['safe_search'] else 'false'}",
            f"time_range = {'true' if e['time_range'] else 'false'}",
            f"wave = {e['wave']}",
        ]
        out.append("  { " + ", ".join(fields) + " },")
    out.append("]")
    out.append("")
    OUT.write_text("\n".join(out), encoding="utf-8")

    waves = {w: sum(1 for e in engines.values() if e["wave"] == w) for w in (1, 2, 3)}
    enabled = sum(1 for e in engines.values() if e["enabled"])
    print(f"OK: wrote {OUT} — 248 engines (waves: {waves}, enabled by default: {enabled})")
    return 0

if __name__ == "__main__":
    sys.exit(main())
