use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SHADCN: &str = "https://github.com/shadcn-ui/ui.git";
const SHADCN_REV: &str = "e31626978fe8e3804fe2e792e11e32dc560e40a8";

const DEPENDENCIES: &str = r#"{
  "private": true,
  "dependencies": {
    "@tailwindcss/language-server": "0.16.0",
    "@tailwindcss/node": "4.3.4",
    "@tailwindcss/oxide": "4.3.4",
    "clsx": "2.1.1",
    "cn": "0.2.6",
    "esbuild": "0.28.2",
    "shadcn": "4.22.1",
    "streamdown": "2.7.0",
    "tailwind-merge": "3.7.0",
    "tailwindcss": "4.3.4",
    "tw-animate-css": "1.4.0"
  }
}
"#;

pub fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace root")
}

pub fn git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git").args(args).current_dir(dir).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("git {} failed", args.join(" "))) }
}

/// What the platform counter reports about the process it wraps.
pub struct Counted {
    pub instructions: u64,
    /// Peak RSS in bytes.
    pub peak_rss: u64,
}

/// One run of `program` under [`counter`], with its stdout.
pub fn count(program: &OsStr, args: &[&OsStr]) -> Result<(Counted, String), String> {
    let (counter, counter_args) = counter().split_first().expect("the counter is a command");
    let output = Command::new(counter).args(counter_args).arg(program).args(args).output().map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(stderr.into_owned());
    }
    Ok((counted(&stderr)?, String::from_utf8_lossy(&output.stdout).into_owned()))
}

/// The command prefix that makes a program report its user-space instructions and peak RSS on stderr, which [`counted`]
/// reads: `time -l` on macOS, `time -f` around `perf stat` elsewhere.
pub fn counter() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &["/usr/bin/time", "-l"]
    } else {
        &["/usr/bin/time", "-f", "%M kB maximum resident set size", "perf", "stat", "-x,", "-e", "instructions:u", "--"]
    }
}

/// Reads the stderr of a process run under [`counter`].
pub fn counted(stderr: &str) -> Result<Counted, String> {
    let instructions = if cfg!(target_os = "macos") {
        stderr.lines().find_map(|line| line.trim().strip_suffix("instructions retired")).and_then(|n| n.trim().parse().ok())
    } else {
        stderr.lines().find(|line| line.contains("instructions")).and_then(|line| line.split(',').next()).and_then(|n| n.parse().ok())
    };
    let peak_rss = stderr.lines().find(|line| line.contains("maximum resident set size")).and_then(|line| {
        let n: u64 = line.split_whitespace().next()?.parse().ok()?;
        Some(if line.contains("kB") { n * 1024 } else { n })
    });
    match (instructions, peak_rss) {
        (Some(instructions), Some(peak_rss)) => Ok(Counted { instructions, peak_rss }),
        _ => Err(format!("no instruction count or peak RSS in:\n{stderr}")),
    }
}

/// Instructions per iteration of a benchmark script that runs setup alone in mode `"0"` and setup plus `k` iterations in
/// mode `"1"`: `min(setup + k iterations) - min(setup)` over `runs` each, divided by `k`. With `k == 0` the whole mode
/// `"1"` run counts once. `run` runs the script in a mode and returns the instructions it counted.
pub fn per_iteration(k: u64, runs: usize, mut run: impl FnMut(&str) -> Result<u64, String>) -> Result<f64, String> {
    let mut setup = if k == 0 { 0 } else { u64::MAX };
    let mut full = u64::MAX;
    for mode in if k == 0 { &["1"][..] } else { &["0", "1"] } {
        for _ in 0..runs {
            let instructions = run(mode)?;
            let best = if *mode == "0" { &mut setup } else { &mut full };
            *best = (*best).min(instructions);
        }
    }
    Ok(full.saturating_sub(setup) as f64 / k.max(1) as f64)
}

/// Installs the pinned Tailwind toolchain and the shadcn docs' CSS dependencies into `deps`, returning its `node_modules`.
pub fn install(deps: &Path) -> Result<PathBuf, String> {
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
pub fn shadcn(work: &Path, modules: &Path) -> Result<(PathBuf, PathBuf), String> {
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

pub fn duration(ms: f64) -> String {
    if ms >= 1.0 { format!("{ms:.1} ms") } else { format!("{:.2} µs", ms * 1e3) }
}

pub fn bytes(n: f64) -> String {
    match n.abs() {
        a if a >= 1048576.0 => format!("{:.1} MB", n / 1048576.0),
        a if a >= 1024.0 => format!("{:.1} KB", n / 1024.0),
        _ => format!("{n:.0} B"),
    }
}

pub fn si(n: f64) -> String {
    match n {
        n if n >= 1e9 => format!("{:.2}G", n / 1e9),
        n if n >= 1e6 => format!("{:.1}M", n / 1e6),
        n if n >= 1e3 => format!("{:.1}k", n / 1e3),
        n => format!("{n:.0}"),
    }
}
