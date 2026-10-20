use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use orisu_allocator::Allocator;
use orisu_compat_tailwind::Tailwind;
use orisu_compiler::compiler::{Options, compile};

use crate::conformance::load_stylesheet;

const USAGE: &str = "usage: cargo xtask bench <input.css> <candidates.txt> [--runs N] [--hmr N] [--against <git-rev>]";

/// Counts the instructions of a release build compiling `input.css` with `candidates.txt`, as the minimum over `--runs`;
/// with `--against`, also for `<git-rev>` built in a worktree, and requires both outputs to match byte for byte.
/// With `--hmr N`, the last `N` candidates are added one build at a time and the count is the cost of one such build.
pub fn run(args: &[String]) -> Result<(), String> {
    let mut positional = Vec::new();
    let mut runs = 5;
    let mut against = None;
    let mut hmr = 0;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--runs" => runs = iter.next().and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or(USAGE)?,
            "--hmr" => hmr = iter.next().and_then(|n| n.parse().ok()).filter(|&n| n > 0).ok_or(USAGE)?,
            "--against" => against = Some(iter.next().ok_or(USAGE)?.clone()),
            _ => positional.push(fs::canonicalize(arg).map_err(|e| format!("{arg}: {e}"))?),
        }
    }
    let [css, candidates] = positional.as_slice() else {
        return Err(USAGE.to_owned());
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask lives in the workspace root");
    let bench = root.join("target/bench");
    fs::create_dir_all(&bench).map_err(|e| e.to_string())?;

    let current = build(root, &root.join("target"))?;
    let (instructions, output) = measure(&current, css, candidates, runs, hmr, &bench.join("current.css"))?;
    println!("current   {instructions:>14} instructions");

    let Some(rev) = against else {
        return Ok(());
    };
    let tree = bench.join(format!("tree-{}", rev.replace('/', "-")));
    let resolved = Command::new("git").args(["rev-parse", "--verify", &format!("{rev}^{{commit}}")]).current_dir(root).output();
    let commit = match resolved {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_owned(),
        _ => return Err(format!("unknown revision {rev}")),
    };
    if !tree.exists() {
        git(root, &["worktree", "add", "--detach", &tree.to_string_lossy(), &commit])?;
    } else {
        git(&tree, &["checkout", "--detach", &commit])?;
    }
    let baseline = build(&tree, &bench.join("target"))?;
    let (base_instructions, base_output) = measure(&baseline, css, candidates, runs, hmr, &bench.join("baseline.css"))?;
    let delta = (instructions as f64 - base_instructions as f64) / base_instructions as f64 * 100.0;
    println!("{rev:<9} {base_instructions:>14} instructions");
    println!("delta     {delta:>+13.2}%");
    if output != base_output {
        return Err(format!("outputs differ: {} vs {}", bench.join("current.css").display(), bench.join("baseline.css").display()));
    }
    Ok(())
}

/// The workload `bench` measures: one compile and one build, written to `out`. With `hmr`, the last `hmr` candidates
/// are left out of that build and, unless `base-only` is given, then added one build at a time.
pub fn workload(args: &[String]) -> Result<(), String> {
    let (css, candidates, out, hmr, mode) = match args {
        [css, candidates, out] => (css, candidates, out, 0, "full"),
        [css, candidates, out, hmr, mode] => {
            (css, candidates, out, hmr.parse().map_err(|_| format!("invalid hmr count {hmr}"))?, mode.as_str())
        }
        _ => return Err("usage: xtask bench-run <input.css> <candidates.txt> <output.css> [<hmr> <full|base-only>]".to_owned()),
    };
    let input = fs::read_to_string(css).map_err(|e| format!("{css}: {e}"))?;
    let candidates = fs::read_to_string(candidates).map_err(|e| format!("{candidates}: {e}"))?;
    let base = Path::new(css).parent().map(|dir| dir.to_string_lossy().into_owned()).unwrap_or_default();
    let alloc = Allocator::new();
    let options =
        Options { base: &base, load_stylesheet: Some(Box::new(load_stylesheet)), extension: Some(&Tailwind), ..Options::default() };
    let mut compiler = compile(&input, &alloc, options).map_err(|e| e.to_string())?;
    let candidates: Vec<&str> = candidates.split_whitespace().collect();
    let (base, added) = candidates.split_at(candidates.len().saturating_sub(hmr));
    let mut css = compiler.build(base.iter().copied()).to_owned();
    if mode == "full" {
        for candidate in added {
            css = compiler.build([*candidate]).to_owned();
        }
    }
    fs::write(out, css).map_err(|e| format!("{out}: {e}"))
}

fn build(dir: &Path, target: &Path) -> Result<PathBuf, String> {
    let status = Command::new("cargo")
        .args(["build", "--release", "--quiet", "-p", "xtask", "--target-dir"])
        .arg(target)
        .current_dir(dir)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("cargo build failed in {}", dir.display()));
    }
    Ok(target.join("release/xtask"))
}

pub(crate) fn git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git").args(args).current_dir(dir).status().map_err(|e| e.to_string())?;
    if status.success() { Ok(()) } else { Err(format!("git {} failed", args.join(" "))) }
}

/// The minimum instruction count over `runs`, and the bytes the workload wrote. With `hmr`, the count is that of one
/// incremental build: the difference to a run that skips the incremental builds, divided by `hmr`.
fn measure(binary: &Path, css: &Path, candidates: &Path, runs: usize, hmr: usize, out: &Path) -> Result<(u64, Vec<u8>), String> {
    let hmr_arg = hmr.to_string();
    let min = |mode: &str| -> Result<u64, String> {
        let mut best = u64::MAX;
        for _ in 0..runs {
            let mut args = vec![OsStr::new("bench-run"), css.as_os_str(), candidates.as_os_str(), out.as_os_str()];
            if hmr > 0 {
                args.extend([OsStr::new(&hmr_arg), OsStr::new(mode)]);
            }
            best = best.min(count(binary.as_os_str(), &args)?.0);
        }
        Ok(best)
    };
    let instructions = if hmr == 0 {
        min("full")?
    } else {
        let setup = min("base-only")?;
        min("full")?.saturating_sub(setup) / hmr as u64
    };
    Ok((instructions, fs::read(out).map_err(|e| e.to_string())?))
}

/// User-space instructions of one run of `program` and its stdout: `perf stat` on Linux, `time -l` on macOS.
pub(crate) fn count(program: &OsStr, args: &[&OsStr]) -> Result<(u64, String), String> {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = Command::new("/usr/bin/time");
        command.arg("-l");
        command
    } else {
        let mut command = Command::new("perf");
        command.args(["stat", "-x,", "-e", "instructions:u", "--"]);
        command
    };
    let output = command.arg(program).args(args).output().map_err(|e| e.to_string())?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        return Err(stderr.into_owned());
    }
    let parsed = if cfg!(target_os = "macos") {
        stderr.lines().find_map(|line| line.trim().strip_suffix("instructions retired")).and_then(|n| n.trim().parse().ok())
    } else {
        stderr.lines().find(|line| line.contains("instructions")).and_then(|line| line.split(',').next()).and_then(|n| n.parse().ok())
    };
    let instructions = parsed.ok_or_else(|| format!("no instruction count in:\n{stderr}"))?;
    Ok((instructions, String::from_utf8_lossy(&output.stdout).into_owned()))
}
