use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::bench::{count, git};

const USAGE: &str = "usage: cargo xtask bench-tailwind [--entry <app.css> --root <dir>] [--runs N] [scenario...]";

const SHADCN: &str = "https://github.com/shadcn-ui/ui.git";
const SHADCN_REV: &str = "e31626978fe8e3804fe2e792e11e32dc560e40a8";

const DEPENDENCIES: &str = r#"{
  "private": true,
  "dependencies": {
    "@tailwindcss/node": "4.3.3",
    "@tailwindcss/oxide": "4.3.3",
    "clsx": "2.1.1",
    "cn": "0.2.6",
    "esbuild": "0.28.2",
    "shadcn": "4.22.1",
    "streamdown": "2.7.0",
    "tailwind-merge": "3.7.0",
    "tailwindcss": "4.3.3",
    "tw-animate-css": "1.4.0"
  }
}
"#;

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

/// Compares orisu with Tailwind 4.3.3 on a real project, shadcn/ui docs at a pinned commit by default. Requires both to produce
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace root");
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

    let config =
        format!("{{\"entry\":{},\"root\":{},\"modules\":{},\"orisu\":{}}}", json(&entry), json(&project), json(&modules), json(&dist));
    fs::write(work.join("config.json"), config).map_err(|e| e.to_string())?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench-tailwind.mjs");
    let status = Command::new("node").arg(&script).arg(&work).arg("prepare").status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("bench-tailwind.mjs prepare failed".to_owned());
    }

    let selected = |scenario: &str| only.is_empty() || only.iter().any(|name| name == scenario);
    println!(
        "{:<20}{:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>11}{:>11}",
        "scenario", "K", "tailwind", "orisu", "ratio", "tailwind", "orisu", "tailwind", "orisu"
    );
    for &(scenario, k, tailwind) in SCENARIOS.iter().filter(|(scenario, ..)| selected(scenario)) {
        let orisu = measure(&script, &work, "orisu", scenario, k, runs)?;
        let tailwind = if tailwind { Some(measure(&script, &work, "tailwind", scenario, k, runs)?) } else { None };
        let tailwind = tailwind.as_ref();
        println!(
            "{scenario:<20}{k:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>11}{:>11}",
            tailwind.map(|t| si(t.instructions)).unwrap_or_default(),
            si(orisu.instructions),
            tailwind.map(|t| format!("{:.1}×", t.instructions / orisu.instructions)).unwrap_or_default(),
            tailwind.map(|t| duration(t.ms)).unwrap_or_default(),
            duration(orisu.ms),
            tailwind.map(|t| bytes(t.rss as f64)).unwrap_or_default(),
            bytes(orisu.rss as f64),
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
    let mut setup = u64::MAX;
    let mut full = u64::MAX;
    let mut ms = f64::INFINITY;
    let mut rss = u64::MAX;
    let mut heap = i64::MAX;
    for mode in ["0", "1"] {
        for _ in 0..runs {
            let args = [
                OsStr::new("--expose-gc"),
                script.as_os_str(),
                work.as_os_str(),
                OsStr::new(engine),
                OsStr::new(scenario),
                OsStr::new(&iterations),
                OsStr::new(mode),
            ];
            let (instructions, stdout) = count(OsStr::new("node"), &args)?;
            if mode == "0" {
                setup = setup.min(instructions);
            } else {
                full = full.min(instructions);
                let malformed = || format!("{engine} {scenario}: expected `ms rss heap`, got {stdout:?}");
                let mut fields = stdout.split_whitespace();
                ms = ms.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
                rss = rss.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
                heap = heap.min(fields.next().and_then(|n| n.parse().ok()).ok_or_else(malformed)?);
            }
        }
    }
    Ok(Sample { instructions: full.saturating_sub(setup) as f64 / k as f64, ms: ms / k as f64, rss, heap })
}

/// Installs the pinned Tailwind toolchain and the shadcn docs' CSS dependencies into `deps`, returning its `node_modules`.
fn install(deps: &Path) -> Result<PathBuf, String> {
    let manifest = deps.join("package.json");
    let modules = deps.join("node_modules");
    if fs::read_to_string(&manifest).ok().as_deref() == Some(DEPENDENCIES) && modules.exists() {
        return Ok(modules);
    }
    fs::create_dir_all(deps).map_err(|e| e.to_string())?;
    fs::write(&manifest, DEPENDENCIES).map_err(|e| e.to_string())?;
    let status = Command::new("npm")
        .args(["install", "--no-audit", "--no-fund", "--loglevel=error"])
        .current_dir(deps)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("npm install failed".to_owned());
    }
    Ok(modules)
}

/// The shadcn docs app at `SHADCN_REV`, sparsely checked out into `work`, with `modules` linked as its `node_modules`.
fn shadcn(work: &Path, modules: &Path) -> Result<(PathBuf, PathBuf), String> {
    let repo = work.join("shadcn");
    let app = repo.join("apps/v4");
    if !app.exists() {
        fs::create_dir_all(&repo).map_err(|e| e.to_string())?;
        git(&repo, &["init", "--quiet"])?;
        git(&repo, &["remote", "add", "origin", SHADCN])?;
        git(&repo, &["sparse-checkout", "set", "apps/v4"])?;
        git(&repo, &["fetch", "--quiet", "--depth", "1", "origin", SHADCN_REV])?;
        git(&repo, &["checkout", "--quiet", "FETCH_HEAD"])?;
    }
    let link = app.join("node_modules");
    if fs::read_link(&link).ok().as_deref() != Some(modules) {
        let _ = fs::remove_file(&link);
        symlink(modules, &link).map_err(|e| format!("{}: {e}", link.display()))?;
    }
    Ok((app.join("app/globals.css"), app))
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

fn canonical(path: &str) -> Result<PathBuf, String> {
    fs::canonicalize(path).map_err(|e| format!("{path}: {e}"))
}

fn json(path: &Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))
}

fn duration(ms: f64) -> String {
    if ms >= 1.0 { format!("{ms:.1} ms") } else { format!("{:.2} µs", ms * 1e3) }
}

fn bytes(n: f64) -> String {
    match n.abs() {
        a if a >= 1048576.0 => format!("{:.1} MB", n / 1048576.0),
        a if a >= 1024.0 => format!("{:.1} KB", n / 1024.0),
        _ => format!("{n:.0} B"),
    }
}

fn si(n: f64) -> String {
    match n {
        n if n >= 1e9 => format!("{:.2}G", n / 1e9),
        n if n >= 1e6 => format!("{:.1}M", n / 1e6),
        n if n >= 1e3 => format!("{:.1}k", n / 1e3),
        n => format!("{n:.0}"),
    }
}
