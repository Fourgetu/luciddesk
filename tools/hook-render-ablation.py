"""Isolated Hook/native ListView and offscreen rendering ablations (Windows)."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import random
import re
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
HOOK = "crates/desktop-hook/src/geometry/cache.rs"
LABEL = "app/src/pane/label.rs"
GPU = "app/src/pane/native_graphics.rs"
RENDER = "app/src/pane/render.rs"
VARIANTS = ["baseline", "no_hit_cache", "no_spatial_filter", "no_damage_cache",
            "no_label_cache", "no_device_reuse", "no_icon_reuse"]


def replace(path, old, new):
    source = path.read_text(encoding="utf-8")
    if source.count(old) != 1:
        raise RuntimeError(f"Expected one mutation anchor in {path}: {old[:80]}")
    path.write_text(source.replace(old, new, 1), encoding="utf-8")


def run(args, cwd, log, env):
    with log.open("w", encoding="utf-8") as stream:
        result = subprocess.run(args, cwd=cwd, env=env, stdout=stream,
                                stderr=subprocess.STDOUT, timeout=600)
    return result.returncode, log.read_text(encoding="utf-8", errors="replace")


def summarize(result):
    """Compare each render capture to the baseline and retain test summaries."""
    baseline = next(r["pixels"] for r in result["runs"]
                    if r["variant"] == "baseline" and r["kind"] == "render_probe")
    if len(baseline) != 3:
        raise RuntimeError("Baseline did not produce all three DPI captures")
    for entry in result["runs"]:
        log = Path(entry["log"]).read_text(encoding="utf-8", errors="replace")
        entry["test_summaries"] = re.findall(r"test result: [^\n]+", log)
        if entry["kind"] == "hook_probe":
            entry["zero_query_guards"] = re.findall(r"ABLATION_GUARD_ZERO (true|false)", log)
            if entry["variant"] == "baseline" and entry["zero_query_guards"] != ["true"] * 3:
                raise RuntimeError("Baseline did not satisfy the original zero-query guards")
        if entry["kind"] == "render_probe":
            entry["pixels_match_baseline"] = entry["pixels"] == baseline
    result["complete"] = len(result["runs"]) == 40
    result["workspace_changed_after_snapshot"] = [
        name for name, digest in result["source_sha256"].items()
        if not (ROOT / name).is_file()
        or hashlib.sha256((ROOT / name).read_bytes()).hexdigest() != digest
    ]
    if not result["complete"]:
        raise RuntimeError("Experiment did not finish all 40 test/probe processes")


def main():
    out = ROOT / "target" / "hook-render-ablation" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    snapshot = out / "snapshot"
    snapshot.mkdir(parents=True)
    render_probe = (ROOT / "tools/ablation/render_probe.rs").read_text(encoding="utf-8")
    (out / "render_probe.rs").write_text(render_probe, encoding="utf-8")
    print(f"Output: {out}", flush=True)
    for name in ["app", "crates"]:
        shutil.copytree(ROOT / name, snapshot / name)
    for name in ["Cargo.toml", "Cargo.lock", "README.md", "CHANGELOG.md"]:
        shutil.copy2(ROOT / name, snapshot / name)
    hashes = {str(p.relative_to(snapshot)): hashlib.sha256(p.read_bytes()).hexdigest()
              for p in snapshot.rglob("*") if p.is_file()}
    result = {"source_sha256": hashes, "git_head": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip(),
              "git_status": subprocess.check_output(["git", "status", "--short"], cwd=ROOT, text=True),
              "rustc": subprocess.check_output(["rustc", "-Vv"], text=True),
              "profile": "debug", "renderer": "warp", "variants": {}, "runs": []}
    env = dict(os.environ, LUCIDPANE_RENDERER="warp")
    env.pop("LUCIDPANE_ICON_TRACE", None)
    target = out / "build"
    binaries = {}
    for name in VARIANTS:
        print(f"Build {name}", flush=True)
        work = out / name
        shutil.copytree(snapshot, work)
        hook_variant = name in VARIANTS[:4]
        render_variant = name == "baseline" or not hook_variant
        if hook_variant:
            probe = work / "crates/desktop-hook/examples/geometry_probe.rs"
            # Only optimization-count assertions become observations. Keep all
            # geometry, identity, selection, invalidation and pixel assertions.
            for old in [
                'assert_eq!(meter.count(), 0, "Warm hit path queried native geometry again");',
                'assert_eq!(meter.count(), 0);',
                'assert_eq!(meter.count(), 0, "Stable local paint re-queried every native icon");',
            ]:
                replace(probe, old, 'println!("ABLATION_GUARD_ZERO {}", meter.count() == 0);')
            replace(probe, 'println!("PERF: 1580 warm icon hits, geometry queries={}, 79-hit batch median={:?} p95={:?}", meter.count(), timings[10], timings[18]);',
                    'println!(r#"ABLATION_METRIC {{"scenario":"hit_1580","queries":{},"p50_ns":{},"p95_ns":{}}}"#, meter.count(), timings[9].as_nanos(), timings[18].as_nanos());')
            replace(probe, 'println!("PERF: repeated local paint geometry queries={}", meter.count());',
                    'println!(r#"ABLATION_METRIC {{"scenario":"warm_paint","queries":{}}}"#, meter.count());')
            replace(probe, '\n                drop(meter);', '''
                meter.reset();
                let mut paint_times = Vec::new();
                for _ in 0..100 {
                    windows_sys::Win32::Graphics::Gdi::InvalidateRect(view, &raw const dirty, 0);
                    let start = std::time::Instant::now();
                    SendMessageW(view, windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT, 0, 0);
                    paint_times.push(start.elapsed().as_nanos());
                }
                paint_times.sort_unstable();
                println!(r#"ABLATION_METRIC {{"scenario":"paint_100","queries":{},"p50_ns":{},"p95_ns":{}}}"#, meter.count(), paint_times[49], paint_times[94]);
                drop(meter);''')
        if render_variant:
            for file, counter in [(LABEL, "ABLATION_LAYOUTS"), (GPU, "ABLATION_CREATIONS"), (RENDER, "ABLATION_UPLOADS")]:
                with (work / file).open("a", encoding="utf-8") as stream:
                    stream.write(f"\n#[cfg(test)]\npub(super) static {counter}: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);\n")
            for file, anchor, counter in [
                (LABEL, "        let max_height = line_height * lines as f32;", "ABLATION_LAYOUTS"),
                (GPU, '        let device = if std::env::var("LUCIDPANE_RENDERER")', "ABLATION_CREATIONS"),
                (RENDER, "                                let pixels = assets::resample(image, size.0, size.1)?;", "ABLATION_UPLOADS"),
            ]:
                replace(work / file, anchor, f"#[cfg(test)]\n{counter}.fetch_add(1, std::sync::atomic::Ordering::Relaxed);\n" + anchor)
            replace(work / RENDER, "    fn sample_model() -> GroupModel {",
                    render_probe + "\n    fn sample_model() -> GroupModel {")
        if name == "no_hit_cache":
            replace(work / HOOK, "let cached = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.cache.hits.clone()));", "let cached: Option<Rc<Spatial<Hit>>> = None;")
        elif name == "no_damage_cache":
            replace(work / HOOK, "let cached = STATE.with(|s| s.borrow().as_ref().and_then(|s| s.cache.damage.clone()));", "let cached: Option<Rc<Spatial<Damage>>> = None;")
        elif name == "no_spatial_filter":
            replace(work / HOOK, "    fn candidates(&self, rect: &RECT) -> Vec<usize> {", "    fn candidates(&self, rect: &RECT) -> Vec<usize> {\n        return (0..self.entries.len()).collect();")
        elif name == "no_label_cache":
            replace(work / LABEL, "        if let Some(value) = cache.borrow_mut().get(&key) {\n            return Ok(value);\n        }\n", "")
        elif name == "no_device_reuse":
            replace(work / GPU, "        if let Some(device) = cached.as_ref() {", "        if let Some(device) = None::<&windows_canvas::GpuDevice> {")
        elif name == "no_icon_reuse":
            replace(work / RENDER, "        self.target = Some((width, height, None, target.clone()));", "        self.images.clear();\n        self.target = Some((width, height, None, target.clone()));")
        entries = []
        if hook_variant:
            entries += [("hook_tests", ["test", "-p", "desktop-hook", "--lib", "--no-run"]),
                        ("hook_probe", ["build", "-p", "desktop-hook", "--example", "geometry_probe"])]
        if render_variant:
            entries += [("render_tests", ["test", "-p", "lucidpane", "--bin", "lucidpane", "--no-run"])]
        binaries[name] = {}
        for kind, args in entries:
            args = ["cargo", *args, "--offline", "--locked", "--target-dir", str(target), "--message-format=json"]
            code, log = run(args, work, out / f"{name}-{kind}-build.log", env)
            if code:
                raise RuntimeError(f"Build failed: {name}/{kind}; inspect {out}")
            artifacts = [json.loads(line) for line in log.splitlines() if line.startswith('{')]
            executable = next(a["executable"] for a in reversed(artifacts) if a.get("executable"))
            dest = work / f"{kind}.exe"
            shutil.copy2(executable, dest)
            binaries[name][kind] = dest
        result["variants"][name] = {"executables": {k: str(v) for k, v in binaries[name].items()}}
    jobs = [(name, repetition) for name in VARIANTS for repetition in range(3)]
    random.Random(20260914).shuffle(jobs)
    for name, repetition in jobs:
        print(f"Run {name} repetition {repetition}", flush=True)
        work = out / name
        for kind, binary in binaries[name].items():
            # Existing tests once; measurements in three fresh processes.
            commands = []
            if kind == "hook_tests" and repetition == 0:
                commands.append((kind, [str(binary), "--test-threads=1"]))
            elif kind == "hook_probe":
                commands.append((kind, [str(binary)]))
            elif kind == "render_tests":
                if repetition == 0:
                    for prefix in ["pane::render::tests::", "pane::label::", "pane::canvas::tests::"]:
                        commands.append((prefix.replace(":", "_"), [str(binary), prefix, "--skip", "ablation_render_measurements", "--test-threads=1"]))
                commands.append(("render_probe", [str(binary), "ablation_render_measurements", "--test-threads=1", "--nocapture"]))
            for label, args in commands:
                folder = work / f"{label}-{repetition}"
                folder.mkdir()
                run_env = dict(env, LUCIDPANE_ABLATION_OUTPUT=str(folder))
                code, log = run(args, folder, folder / "run.log", run_env)
                metrics = [json.loads(m) for m in re.findall(r"ABLATION_METRIC (\{[^\n]+\})", log)]
                result["runs"].append({"variant": name, "repetition": repetition, "kind": label,
                                       "exit_code": code, "command": args, "metrics": metrics,
                                       "log": str(folder / "run.log"),
                                       "pixels": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in folder.glob("*.bgra")}})
                (out / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
                expected = 3 if label == "hook_probe" else 10 if label == "render_probe" else 0
                if len(metrics) != expected or (name == "baseline" and code):
                    raise RuntimeError(f"Incomplete/baseline failure: {folder / 'run.log'}")
    summarize(result)
    (out / "results.json").write_text(json.dumps(result, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"Finished: {out / 'results.json'}", flush=True)


if __name__ == "__main__":
    main()
