"""Run storage guard ablations in isolated source copies using cached Cargo dependencies."""

import datetime
import hashlib
import json
from pathlib import Path
import re
import shutil
import subprocess
import sys


ROOT = Path(__file__).resolve().parents[1]
STORE = Path("crates/desktop-storage/src/store")
VALUE_GUARD = """        if updates.iter().all(|(k, v)| self.values.get(*k) == Some(v)) {
            return Ok(());
        }
"""
SOURCE_GUARD = """        if source == self.source {
            return Ok(());
        }
"""
SQL_GUARD = "\n             WHERE metadata.value != excluded.value"
VARIANTS = {
    "baseline": [],
    "without_value_guard": [("config.rs", VALUE_GUARD)],
    "without_source_guard": [("config.rs", SOURCE_GUARD)],
    "without_both_config_guards": [
        ("config.rs", VALUE_GUARD), ("config.rs", SOURCE_GUARD)
    ],
    "without_metadata_guard": [("mod.rs", SQL_GUARD)],
}

# Compiled only into the isolated copy. Calls the actual storage implementation.
PROBE = r'''
#[test]
fn ablation_measure_repeated_saves() {
    use std::time::Instant;
    for scenario in ["config_noop", "config_equivalent", "metadata_noop", "workspace_noop", "config_changed"] {
        for repetition in 0..3 {
            let (_dir, mut store) = open();
            let mut workspace = store.load_workspace().unwrap();
            workspace.add_panel(Panel::new(PanelId::new(1), "Ablation", RectDip::new(0.0, 0.0, 300.0, 200.0))).unwrap();
            store.save_workspace(&workspace).unwrap();
            store.save_preference("search_enabled", "1").unwrap();
            store.save_preference("ablation_probe", "same").unwrap();
            let options = workspace.pane_options();
            // Accepted alternate numeric spelling: different input, same TOML value.
            let equivalent = format!("{:.3}|{}|{}|auto|{}|{:.3}", options.corner_radius, options.border, options.snap, options.text_protection, options.grid_scale);
            let before = store.change_count();
            let mut times = Vec::new();
            for iteration in 0..50 {
                let started = Instant::now();
                match scenario {
                    "config_noop" => store.save_preference("search_enabled", "1").unwrap(),
                    "config_equivalent" => store.save_preference("pane_options", &equivalent).unwrap(),
                    "metadata_noop" => store.save_preference("ablation_probe", "same").unwrap(),
                    "workspace_noop" => store.save_workspace(&workspace).unwrap(),
                    "config_changed" => store.save_preference("search_enabled", if iteration % 2 == 0 { "0" } else { "1" }).unwrap(),
                    _ => unreachable!(),
                }
                times.push(started.elapsed().as_nanos());
            }
            times.sort_unstable();
            println!("ABLATION_METRIC {{\"scenario\":\"{scenario}\",\"repetition\":{repetition},\"operations\":50,\"changes\":{},\"p50_ns\":{},\"p95_ns\":{}}}", store.change_count() - before, times[24], times[47]);
            assert_eq!(store.preference("search_enabled").unwrap().as_deref(), Some("1"));
            assert_eq!(store.preference("ablation_probe").unwrap().as_deref(), Some("same"));
            assert_eq!(store.load_workspace().unwrap().panels().len(), 1);
            assert_eq!(store.load_workspace().unwrap().pane_options(), options);
        }
    }
}
'''


def command(args, cwd=ROOT):
    return subprocess.run(args, cwd=cwd, stdout=subprocess.PIPE,
                          stderr=subprocess.STDOUT, encoding="utf-8", errors="replace")


def main():
    stamp = datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    output = ROOT / "target" / "storage-ablation" / stamp
    output.mkdir(parents=True)
    manifest = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    manifest, count = re.subn(
        r"members = \[.*?\]",
        'members = ["crates/desktop-core", "crates/desktop-storage"]',
        manifest, count=1, flags=re.S,
    )
    if count != 1:
        raise RuntimeError("Workspace members block was not found")
    # Take one snapshot so every variant uses identical inputs even if work continues.
    snapshot = output / "snapshot"
    snapshot.mkdir()
    (snapshot / "Cargo.toml").write_text(manifest, encoding="utf-8")
    shutil.copy2(ROOT / "Cargo.lock", snapshot / "Cargo.lock")
    for crate in ["desktop-core", "desktop-storage"]:
        shutil.copytree(ROOT / "crates" / crate, snapshot / "crates" / crate)
    # Prune the copied workspace lockfile offline, then lock all variant builds.
    metadata = command(["cargo", "metadata", "--offline", "--format-version=1"], snapshot)
    if metadata.returncode:
        raise RuntimeError(metadata.stdout)
    fingerprints = {
        str(p.relative_to(snapshot)): hashlib.sha256(p.read_bytes()).hexdigest()
        for p in sorted(snapshot.rglob("*")) if p.is_file()
    }
    results = {
        "git_head": command(["git", "rev-parse", "HEAD"]).stdout.strip(),
        "git_status": command(["git", "status", "--short"]).stdout,
        "rustc": command(["rustc", "-Vv"]).stdout,
        "profile": "debug; timings are diagnostic, not release performance claims",
        "snapshot_sha256": fingerprints,
        "variants": [],
    }
    for name, removals in VARIANTS.items():
        print(f"Running {name}", flush=True)
        work = output / name
        shutil.copytree(snapshot, work)
        for filename, guard in removals:
            path = work / STORE / filename
            source = path.read_text(encoding="utf-8")
            if source.count(guard) != 1:
                raise RuntimeError(f"Expected exactly one guard in {path}")
            path.write_text(source.replace(guard, "", 1), encoding="utf-8")
        probe_file = work / STORE / "config_tests.rs"
        with probe_file.open("a", encoding="utf-8") as stream:
            stream.write(PROBE)
        args = ["cargo", "test", "-p", "desktop-storage", "--lib", "--offline",
                "--locked", "--target-dir", str(ROOT / "target" / "storage-ablation-build"),
                "--", "--test-threads=1", "--nocapture"]
        run = command(args, work)
        (output / f"{name}.log").write_text(run.stdout, encoding="utf-8")
        summary = re.search(r"test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored", run.stdout)
        metrics = [json.loads(m) for m in re.findall(r"ABLATION_METRIC (\{[^\n]+\})", run.stdout)]
        if summary is None or len(metrics) != 15 or run.returncode not in (0, 101):
            raise RuntimeError(f"Incomplete experiment: inspect {output / (name + '.log')}")
        if name == "baseline" and run.returncode:
            raise RuntimeError(f"Baseline failed: inspect {output / 'baseline.log'}")
        result = {"name": name, "command": args, "exit_code": run.returncode,
                  "passed": int(summary[2]), "failed": int(summary[3]),
                  "ignored": int(summary[4]), "metrics": metrics,
                  "failed_tests": re.findall(r"^test (\S+) \.\.\. FAILED$", run.stdout, re.M)}
        # With --nocapture, panic text can interrupt the normal test status line.
        if not result["failed_tests"] and result["failed"]:
            tail = run.stdout.rsplit("failures:\n", 1)[-1]
            result["failed_tests"] = re.findall(r"^    (store::\S+)$", tail, re.M)
        results["variants"].append(result)
        (output / "results.json").write_text(json.dumps(results, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"  {result['passed']} passed, {result['failed']} failed", flush=True)
    print(f"Results: {output / 'results.json'}", flush=True)


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError) as error:
        print(error, file=sys.stderr)
        sys.exit(1)
