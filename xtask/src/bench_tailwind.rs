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
    ("cn", 20000, true),
    ("cn-once", 1, true),
];

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
            name if SCENARIOS.iter().any(|(scenario, ..)| *scenario == name) => only.push(name.to_owned()),
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

    println!("{:<20}{:>7}{:>11}{:>11}{:>8}{:>13}{:>13}", "scenario", "K", "tailwind", "orisu", "ratio", "tailwind", "orisu");
    for &(scenario, k, tailwind) in SCENARIOS {
        if !only.is_empty() && !only.iter().any(|name| name == scenario) {
            continue;
        }
        let orisu = measure(&script, &work, "orisu", scenario, k, runs)?;
        let tailwind = if tailwind { Some(measure(&script, &work, "tailwind", scenario, k, runs)?) } else { None };
        let ratio = tailwind.map(|(instructions, _)| format!("{:.1}×", instructions / orisu.0)).unwrap_or_default();
        println!(
            "{scenario:<20}{k:>7}{:>11}{:>11}{ratio:>8}{:>13}{:>13}",
            tailwind.map(|(instructions, _)| si(instructions)).unwrap_or_default(),
            si(orisu.0),
            tailwind.map(|(_, ms)| duration(ms)).unwrap_or_default(),
            duration(orisu.1),
        );
    }
    Ok(())
}

/// Instructions and wall milliseconds per iteration, each the minimum over `runs`.
fn measure(script: &Path, work: &Path, engine: &str, scenario: &str, k: u64, runs: usize) -> Result<(f64, f64), String> {
    let iterations = k.to_string();
    let mut setup = u64::MAX;
    let mut full = u64::MAX;
    let mut ms = f64::INFINITY;
    for mode in ["0", "1"] {
        for _ in 0..runs {
            let args =
                [script.as_os_str(), work.as_os_str(), OsStr::new(engine), OsStr::new(scenario), OsStr::new(&iterations), OsStr::new(mode)];
            let (instructions, stdout) = count(OsStr::new("node"), &args)?;
            if mode == "0" {
                setup = setup.min(instructions);
            } else {
                full = full.min(instructions);
                ms = ms.min(stdout.trim().parse().map_err(|_| format!("{engine} {scenario}: no timing in {stdout:?}"))?);
            }
        }
    }
    Ok((full.saturating_sub(setup) as f64 / k as f64, ms / k as f64))
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

fn si(n: f64) -> String {
    match n {
        n if n >= 1e9 => format!("{:.2}G", n / 1e9),
        n if n >= 1e6 => format!("{:.1}M", n / 1e6),
        n if n >= 1e3 => format!("{:.1}k", n / 1e3),
        n => format!("{n:.0}"),
    }
}
