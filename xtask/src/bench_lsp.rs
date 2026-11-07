use std::fs;
use std::path::Path;
use std::process::Command;

use crate::harness::{bytes, counted, counter, duration, install, per_iteration, root, shadcn, si};

const USAGE: &str = "usage: cargo xtask bench-lsp [--runs N] [scenario...]";

/// The document every request targets, relative to the shadcn docs app.
const DOCUMENT: &str = "registry/new-york-v4/ui/sidebar.tsx";

/// Scenario and requests per measured run; `startup` measures the whole session up to the first completion.
const SCENARIOS: &[(&str, u64)] =
    &[("startup", 0), ("completion", 20), ("completion-resolve", 200), ("hover", 200), ("diagnostics", 20), ("colors", 50)];

const ENGINES: [&str; 2] = ["tailwind", "orisu"];

struct Sample {
    instructions: f64,
    ms: f64,
    rss: u64,
}

/// Compares `orisu-lsp` with `@tailwindcss/language-server` on the shadcn/ui docs, both driven over stdio by `bench-lsp.mjs`.
/// Instructions and peak RSS are the server process's own; per request, instructions are `min(setup + K requests) - min(setup)`.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut runs = 5;
    let mut only = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--runs" => runs = iter.next().and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or(USAGE)?,
            name if SCENARIOS.iter().any(|(scenario, _)| *scenario == name) => only.push(name.to_owned()),
            _ => return Err(USAGE.to_owned()),
        }
    }
    let root = root();
    let work = root.join("target/bench-tailwind");
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let modules = install(&work.join("deps"))?;
    let (_, project) = shadcn(&work, &modules)?;

    let status = Command::new("cargo")
        .args(["build", "--release", "--quiet", "-p", "orisu_lsp"])
        .current_dir(root)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err("building orisu-lsp failed".to_owned());
    }
    let orisu = root.join("target/release/orisu-lsp");
    let tailwind = modules.join("@tailwindcss/language-server/bin/tailwindcss-language-server");
    let config = serde_json::json!({
        "root": project,
        "file": project.join(DOCUMENT),
        "counter": counter(),
        "servers": { "orisu": [orisu], "tailwind": ["node", tailwind, "--stdio"] },
    });
    fs::write(work.join("lsp.json"), config.to_string()).map_err(|e| e.to_string())?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("bench-lsp.mjs");
    for engine in ENGINES {
        let output =
            Command::new("node").arg(&script).arg(&work).args([engine, "prepare", "0", "1"]).output().map_err(|e| e.to_string())?;
        if !output.status.success() {
            return Err(format!("bench-lsp.mjs {engine} prepare failed:\n{}", String::from_utf8_lossy(&output.stderr)));
        }
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }

    println!(
        "{:<20}{:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>8}{:>11}{:>11}{:>8}",
        "scenario", "K", "tailwind", "orisu", "ratio", "tailwind", "orisu", "ratio", "tailwind", "orisu", "ratio"
    );
    for &(scenario, k) in SCENARIOS.iter().filter(|(scenario, _)| only.is_empty() || only.iter().any(|name| name == scenario)) {
        let [tailwind, orisu] = ENGINES.map(|engine| measure(&script, &work, engine, scenario, k, runs));
        let (tailwind, orisu) = (tailwind?, orisu?);
        println!(
            "{scenario:<20}{k:>7}{:>11}{:>11}{:>8}{:>13}{:>13}{:>8}{:>11}{:>11}{:>8}",
            si(tailwind.instructions),
            si(orisu.instructions),
            format!("{:.1}×", tailwind.instructions / orisu.instructions),
            duration(tailwind.ms),
            duration(orisu.ms),
            format!("{:.1}×", tailwind.ms / orisu.ms),
            bytes(tailwind.rss as f64),
            bytes(orisu.rss as f64),
            format!("{:.1}×", tailwind.rss as f64 / orisu.rss as f64),
        );
    }
    Ok(())
}

/// Per request, or for `startup` per session: server instructions and wall milliseconds, each the minimum over `runs`, and the
/// server's peak RSS, the minimum over `runs`.
fn measure(script: &Path, work: &Path, engine: &str, scenario: &str, k: u64, runs: usize) -> Result<Sample, String> {
    let iterations = k.to_string();
    let mut ms = f64::INFINITY;
    let mut rss = u64::MAX;
    let instructions = per_iteration(k, runs, |mode| {
        let output =
            Command::new("node").arg(script).arg(work).args([engine, scenario, &iterations, mode]).output().map_err(|e| e.to_string())?;
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !output.status.success() {
            return Err(format!("bench-lsp.mjs {engine} {scenario} failed:\n{stderr}"));
        }
        let counted = counted(&stderr)?;
        if mode == "1" {
            let stdout = String::from_utf8_lossy(&output.stdout);
            ms = ms.min(stdout.trim().parse().map_err(|_| format!("{engine} {scenario}: expected `ms`, got {stdout:?}"))?);
            rss = rss.min(counted.peak_rss);
        }
        Ok(counted.instructions)
    })?;
    Ok(Sample { instructions, ms: ms / k.max(1) as f64, rss })
}
