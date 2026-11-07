//! A language server for orisu and Tailwind CSS v4 projects, speaking LSP over stdio.

mod color;
mod context;
mod document;
mod features;
mod lint;
mod project;
mod server;
mod settings;

pub fn run() -> Result<(), String> {
    server::run().map_err(|error| error.to_string())
}

/// The version of the `orisu` npm package, which ships the server.
pub fn version() -> String {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../../../packages/orisu/package.json")).expect("valid package.json");
    manifest["version"].as_str().expect("package.json has a version").to_owned()
}

/// Lints `files` against the project compiled from `entry` with [`settings::Settings::check`]; with `fix`, rewrites each file with
/// its fixes.
pub fn check(entry: &std::path::Path, files: &[std::path::PathBuf], fix: bool) -> Result<Vec<orisu_diagnostics::Diagnostic>, String> {
    let project = project::Project::load(entry).map_err(|error| error.message)?;
    let settings = settings::Settings::check();
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("Could not read `{}`: {e}", file.display()))?;
        let name = file.to_string_lossy();
        let language = document::Language::detect("", &name);
        let scan = lint::scan(&project, &settings, &text, language);
        let reports = lint::lint(&project, &settings, &scan, lint::Pass::Check).into_iter().map(|finding| finding.report).collect();
        let diagnostics = orisu_diagnostics::resolve(reports, &text, &name);
        if fix && diagnostics.iter().any(|d| !d.fixes.is_empty()) {
            let fixed = orisu_diagnostics::apply_fixes(&text, &diagnostics);
            std::fs::write(file, fixed).map_err(|e| format!("Could not write `{}`: {e}", file.display()))?;
        }
        out.extend(diagnostics);
    }
    Ok(out)
}
