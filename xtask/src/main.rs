mod bench;
mod bench_tailwind;
mod check_core;
mod codegen;
mod conformance;

const USAGE: &str = "usage:
  cargo xtask codegen [--check]
  cargo xtask check-core
  cargo xtask conformance <dir> [--tailwind <node_modules>]
  cargo xtask bench <input.css> <candidates.txt> [--runs N] [--hmr N] [--against <git-rev>]
  cargo xtask bench-tailwind [--entry <app.css> --root <dir>] [--runs N] [scenario...]";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("codegen") => codegen::run(&args[1..]),
        Some("check-core") => check_core::run(),
        Some("conformance") => conformance::run(&args[1..]),
        Some("bench") => bench::run(&args[1..]),
        Some("bench-tailwind") => bench_tailwind::run(&args[1..]),
        Some("bench-run") => bench::workload(&args[1..]),
        _ => Err(USAGE.to_owned()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
