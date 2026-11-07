use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::harness::{bytes, count, duration, install, per_iteration, root, shadcn, si};

const USAGE: &str = "usage: cargo xtask bench-tailwind [--entry <app.css> --root <dir>] [--runs N] [scenario...]";

/// Scenario, iterations per measured run, and whether Tailwind has a counterpart.
const SCENARIOS: &[(&str, u64, bool)] = &[
    ("compile", 5, true),
    ("scan", 5, true),
    ("scan-cached", 5, false),
    ("build", 5, true),
    ("cold-start", 3, true),
    ("hmr", 50, true),
    ("hmr-no-new-class", 200, true),
    ("canonicalize-cold", 3, true),
    ("canonicalize-warm", 50, true),
];

/// `cn` scenarios and iterations per measured run, compared across `CN_ENGINES`.
const CN_SCENARIOS: &[(&str, u64)] = &[("cn", 20000), ("cn-once", 1)];

/// Engine id in `bench-tailwind.mjs` and its column title.
const CN_ENGINES: &[(&str, &str)] = &[("tailwind", "clsx+tw-merge"), ("cn", "cn"), ("orisu", "orisu/cn")];

struct Sample {
    instructions: f64,
    ms: f64,
    rss: u64,
    heap: i64,
}

/// Compares orisu with Tailwind 4.3.4 on a real project, shadcn/ui docs at a pinned commit by default. Requires both to produce
/// the same candidates and CSS, then prints instructions per iteration as `min(setup + K iterations) - min(setup)` over `--runs`.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut entry = None;
    let mut project = None;
    let mut runs = 5;
    let mut only = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--entry" => entry = Some(canonical(iter.next().ok_or(USAGE)?)?),
            "--root" => project = Some(canonical(iter.next().ok_or(USAGE)?)?),
            "--runs" => runs = iter.next().and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or(USAGE)?,
            name if SCENARIOS.iter().any(|(scenario, ..)| *scenario == name)
                || CN_SCENARIOS.iter().any(|(scenario, _)| *scenario == name) =>
            {
                only.push(name.to_owned())
            }
            _ => return Err(USAGE.to_owned()),
        }
    }
    let root = root();
    let work = root.join("target/bench-tailwind");
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let modules = install(&work.join("deps"))?;
    let (entry, project) = match (entry, project) {
        (Some(entry), Some(project)) => (entry, project),
        (None, None) => shadcn(&work, &modules)?,
        _ => return Err(USAGE.to_owned()),
    };

    let package = root.join("packages/orisu");
    let status = Command::new("node").arg("scripts/build-native.ts").current_dir(&package).status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("building orisu_napi failed".to_owned());
    }
    let dist = package.join("dist");
    if !dist.join("index.js").exists() {
        return Err(format!("{} is missing: run `npm run build` in {}", dist.display(), package.display()));
    }

    let path = |path: &Path| path.to_string_lossy().into_owned();
    let config = serde_json::json!({ "entry": path(&entry), "root": path(&project), "modules": path(&modules), "orisu": path(&dist) });
    fs::write(work.join("config.json"), config.to_string()).map_err(|e| e.to_string())?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench-tailwind.mjs");
    let status = Command::new("node").arg(&script).arg(&work).arg("prepare").status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("bench-tailwind.mjs prepare failed".to_owned());
    }

    let selected = |scenario: &str| only.is_empty() || only.iter().any(|name| name == scenario);
    println!(
        "{:<20}{:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>8}{:>11}{:>11}{:>8}",
        "scenario", "K", "tailwind", "orisu", "ratio", "tailwind", "orisu", "ratio", "tailwind", "orisu", "ratio"
    );
    for &(scenario, k, tailwind) in SCENARIOS.iter().filter(|(scenario, ..)| selected(scenario)) {
        let orisu = measure(&script, &work, "orisu", scenario, k, runs)?;
        let tailwind = if tailwind { Some(measure(&script, &work, "tailwind", scenario, k, runs)?) } else { None };
        let tailwind = tailwind.as_ref();
        println!(
            "{scenario:<20}{k:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>8}{:>11}{:>11}{:>8}",
            tailwind.map(|t| si(t.instructions)).unwrap_or_default(),
            si(orisu.instructions),
            tailwind.map(|t| format!("{:.1}×", t.instructions / orisu.instructions)).unwrap_or_default(),
            tailwind.map(|t| duration(t.ms)).unwrap_or_default(),
            duration(orisu.ms),
            tailwind.map(|t| format!("{:.1}×", t.ms / orisu.ms)).unwrap_or_default(),
            tailwind.map(|t| bytes(t.rss as f64)).unwrap_or_default(),
            bytes(orisu.rss as f64),
            tailwind.map(|t| format!("{:.1}×", t.rss as f64 / orisu.rss as f64)).unwrap_or_default(),
        );
    }

    let cn: Vec<_> = CN_SCENARIOS.iter().filter(|(scenario, _)| selected(scenario)).collect();
    if cn.is_empty() {
        return Ok(());
    }
    let row =
        |label: &str, cells: Vec<String>| println!("{label:<20}{}", cells.iter().map(|cell| format!("{cell:>15}")).collect::<String>());
    println!();
    row("cn", CN_ENGINES.iter().map(|(_, title)| title.to_string()).collect());
    for &&(scenario, k) in &cn {
        let samples =
            CN_ENGINES.iter().map(|(engine, _)| measure(&script, &work, engine, scenario, k, runs)).collect::<Result<Vec<_>, _>>()?;
        row(&format!("{scenario} instructions"), samples.iter().map(|s| si(s.instructions)).collect());
        row(&format!("{scenario} time"), samples.iter().map(|s| duration(s.ms)).collect());
        row(&format!("{scenario} heap"), samples.iter().map(|s| bytes(s.heap as f64)).collect());
    }
    let output = Command::new("node").arg(&script).arg(&work).arg("bundle").output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("bench-tailwind.mjs bundle failed:\n{}", String::from_utf8_lossy(&output.stderr)));
    }
    let sizes: Vec<Vec<f64>> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.split(' ').skip(1).filter_map(|n| n.parse().ok()).collect())
        .collect();
    row("bundle min", sizes.iter().map(|size| bytes(size[0])).collect());
    row("bundle gzip", sizes.iter().map(|size| bytes(size[1])).collect());
    Ok(())
}

/// Per iteration: instructions and wall milliseconds, each the minimum over `runs`. Per process: peak RSS and JS heap retained
/// after setup and all iterations, each the minimum over `runs`.
fn measure(script: &Path, work: &Path, engine: &str, scenario: &str, k: u64, runs: usize) -> Result<Sample, String> {
    let iterations = k.to_string();
    let mut ms = f64::INFINITY;
    let mut rss = u64::MAX;
    let mut heap = i64::MAX;
    let instructions = per_iteration(k, runs, |mode| {
        let args = [
            OsStr::new("--expose-gc"),
            script.as_os_str(),
            work.as_os_str(),
            OsStr::new(engine),
            OsStr::new(scenario),
            OsStr::new(&iterations),
            OsStr::new(mode),
        ];
        let (counted, stdout) = count(OsStr::new("node"), &args)?;
        if mode == "1" {
            let malformed = || format!("{engine} {scenario}: expected `ms rss heap`, got {stdout:?}");
            let mut fields = stdout.split_whitespace();
            ms = ms.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
            rss = rss.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
            heap = heap.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
        }
        Ok(counted.instructions)
    })?;
    Ok(Sample { instructions, ms: ms / k.max(1) as f64, rss, heap })
}

fn canonical(path: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))
}
