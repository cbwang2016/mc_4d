#!/usr/bin/env python3
"""Measure static rendering scenarios; this does not certify gameplay minimum FPS.

Run after `cargo build --release`: python3 scripts/benchmark.py --frames 300
Outputs and GPU driver caches stay under this repository's target/performance.
The report includes the binary hash so results cannot be mistaken for a later build.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]
SCENES = {
    "aligned": ([], 2),
    "rotated": (["--ana", "37", "--right-ana", "23"], 2),
    "rotated_ghosts": (["--ana", "37", "--right-ana", "23", "--ghosts"], 2),
    "rotated_ghosts_shadows": (["--ana", "37", "--right-ana", "23", "--ghosts"], 2),
    "ground": (["--pitch", "-60"], 2),
    "sky": (["--pos", "0.5,140,0.5,0.5", "--pitch", "30"], 2),
    "solid_interior": (["--pos", "0.5,20,0.5,0.5", "--ana", "37"], 2),
    "four_views": (["--ana", "37", "--right-ana", "23"], 4),
    "six_views": (["--ana", "37", "--right-ana", "23"], 6),
    "nine_views_ghosts": (["--ana", "37", "--right-ana", "23", "--ghosts"], 9),
}

# Seed 1337 fixtures from the core biome_locations example, at least 32 blocks
# inside each biome. Regenerate when terrain generation intentionally changes.
BIOMES = {
    "plains": "-47.5,55,0.5,0.5",
    "forest": "0.5,61,-47.5,0.5",
    "desert": "-95.5,63,-95.5,0.5",
    "taiga": "96.5,50,96.5,0.5",
    "mountains": "0.5,100,-239.5,0.5",
    "swamp": "96.5,48,-47.5,128.5",
    "ocean": "-191.5,47,336.5,-383.5",
}
for biome, position in BIOMES.items():
    SCENES[f"biome_{biome}"] = (["--pos", position, "--yaw", "45", "--pitch", "-15"], 2)
for biome in ("forest", "mountains"):
    SCENES[f"biome_{biome}_ghosts"] = ([*SCENES[f"biome_{biome}"][0], "--ana", "37", "--right-ana", "23", "--ghosts"], 2)
SCENES["underwater_ghosts"] = (["--pos", "-191.5,40,336.5,-383.5", "--yaw", "45", "--pitch", "-15", "--ana", "37", "--ghosts"], 2)
SCENES["rain_ghosts"] = ([*SCENES["biome_forest_ghosts"][0], "--command", "/weather rain"], 2)
SCENES["rain_ghosts_shadows"] = SCENES["rain_ghosts"]
SCENES["thunder_nine_views"] = (["--ana", "37", "--right-ana", "23", "--command", "/weather thunder"], 9)
SCENES["snow_taiga"] = ([*SCENES["biome_taiga"][0], "--command", "/weather rain"], 2)
SCENES["night_forest"] = ([*SCENES["biome_forest"][0], "--command", "/time set midnight"], 2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--frames", type=int, default=300)
    parser.add_argument("--size", default="1920x1080")
    parser.add_argument("--distance", type=int, choices=range(2, 7), default=5)
    parser.add_argument("--scene", action="append", choices=SCENES, help="repeat to select scenes (default: all)")
    parser.add_argument("--binary", type=Path, default=ROOT / "target/release/mc4d")
    args = parser.parse_args()
    if args.frames < 1:
        parser.error("--frames must be positive")
    binary = args.binary.resolve()
    output = ROOT / "target/performance"
    data = output / "static-data"
    cache = output / "cache"
    tmp = output / "tmp"
    for directory in (data, cache, tmp):
        directory.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(XDG_CACHE_HOME=str(cache), MESA_SHADER_CACHE_DIR=str(cache), TMPDIR=str(tmp))
    report = {
        "time_utc": datetime.now(timezone.utc).isoformat(),
        "platform": platform.platform(),
        "minimum_fps_target": 20.0,
        "frame_budget_ms": 1000.0 / 20.0,
        "binary": str(binary),
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "scope": "Static headless rendering, including GPU completion; excludes gameplay updates, streaming during motion, window presentation, and startup. No all-scenes FPS guarantee.",
        "scenes": [],
    }
    pattern = re.compile(
        r"mean ([\d.]+) ms \(([\d.]+) FPS\), p95 ([\d.]+) ms, p99 ([\d.]+) ms, "
        r"worst ([\d.]+) ms \(minimum ([\d.]+) FPS\)"
    )
    failed = []
    over_budget = []
    for name, (options, views) in SCENES.items():
        if args.scene and name not in args.scene:
            continue
        # Explicit settings make these independent of the user's saves/preferences.
        (data / "settings.toml").write_text(
            f"render_scale = 1.0\nrender_distance = {args.distance}\nshadows = {'true' if name.endswith('_shadows') else 'false'}\ninset = true\nghosts = false\n"
            f"view_count = {views}\n",
            encoding="utf-8",
        )
        command = [str(binary), "--bench", str(args.frames), "--size", args.size, "--debug",
                   "--data-dir", str(data), *options]
        print(f"Measuring {name} ({args.frames} frames)...", flush=True)
        result = subprocess.run(command, cwd=ROOT, env=env, text=True, capture_output=True, timeout=300)
        entry = {"name": name, "views_requested": views, "command": command, "returncode": result.returncode,
                 "stdout": result.stdout, "stderr": result.stderr}
        match = pattern.search(result.stdout)
        biome_match = re.search(r"Scene biome: (\w+);", result.stdout)
        if biome_match:
            entry["actual_biome"] = biome_match[1]
        expected_biome = next((biome for biome in BIOMES if name.startswith(f"biome_{biome}")), None)
        if name.startswith("rain_ghosts") or name == "night_forest":
            expected_biome = "forest"
        if name == "snow_taiga":
            expected_biome = "taiga"
        if name == "underwater_ghosts":
            expected_biome = "ocean"
        if expected_biome and (not biome_match or biome_match[1].lower() != expected_biome):
            entry["fixture_error"] = f"Expected {expected_biome}; regenerate biome coordinates for this terrain version"
        environment = re.search(r"Scene environment: ticks (\d+), rain ([\d.]+), thunder (true|false), daylight ([\d.]+); lighting ready (true|false), light GPU bytes (\d+)", result.stdout)
        if environment:
            entry["environment"] = {"ticks": int(environment[1]), "rain": float(environment[2]),
                                    "thunder": environment[3] == "true", "daylight": float(environment[4]),
                                    "lighting_ready": environment[5] == "true", "light_gpu_bytes": int(environment[6])}
            e = entry["environment"]
            if not e["lighting_ready"] or (name.startswith(("rain_", "thunder_", "snow_")) and e["rain"] < 0.99) or (name.startswith("thunder_") and not e["thunder"]) or (name.startswith("night_") and e["daylight"] > 0.01):
                entry["fixture_error"] = "Requested weather/time or completed lighting was not present"
        elif result.returncode == 0:
            entry["fixture_error"] = "Binary does not report lighting and environment readiness"
        if match:
            entry["measurements"] = dict(zip(
                ("mean_ms", "mean_fps", "p95_ms", "p99_ms", "worst_ms", "minimum_fps"),
                map(float, match.groups()),
            ))
        slow = re.search(r"budget: (\d+)/(\d+)", result.stdout)
        if slow:
            entry["frames_over_budget"] = int(slow[1])
            entry["frames_measured"] = int(slow[2])
            entry["meets_static_frame_budget"] = int(slow[1]) == 0
            if int(slow[1]) > 0:
                over_budget.append(name)
        report["scenes"].append(entry)
        (output / "static-scenes.json").write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
        print(result.stdout or result.stderr, flush=True)
        if result.returncode or not match or "fixture_error" in entry:
            failed.append(name)
    print(f"Report: {output / 'static-scenes.json'}")
    if failed:
        raise SystemExit(f"Benchmarks failed or lacked valid timing metrics: {', '.join(failed)}")
    if over_budget:
        raise SystemExit(f"Static frames exceeded the 50 ms / 20 FPS budget: {', '.join(over_budget)}")


if __name__ == "__main__":
    main()
