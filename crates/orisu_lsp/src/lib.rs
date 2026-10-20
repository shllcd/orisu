//! A language server for orisu and Tailwind CSS v4 projects, speaking LSP over stdio.

mod color;
mod context;
mod document;
mod features;
mod lint;
mod project;
mod resolve;
mod server;
mod settings;

/// Serves one client over stdio until it shuts down.
pub fn run() -> Result<(), String> {
    server::run().map_err(|error| error.to_string())
}

/// Lints `files` against the project compiled from `entry` with the default settings, plus unknown variants and misspelled
/// utilities; with `fix`, rewrites each file with its fixes.
pub fn check(entry: &std::path::Path, files: &[std::path::PathBuf], fix: bool) -> Result<Vec<orisu_diagnostics::Diagnostic>, String> {
    let project = project::Project::load(entry)?;
    let mut settings = settings::Settings::default();
    settings.lint.unknown_classes = settings::Level::Error;
    let mut out = Vec::new();
    for file in files {
        let text = std::fs::read_to_string(file).map_err(|e| format!("Could not read `{}`: {e}", file.display()))?;
        let name = file.to_string_lossy();
        let language = document::Language::detect("", &name);
        let reports = lint::lint(&project, &settings, &text, language, true)
            .into_iter()
            .map(|finding| finding.report)
            .filter(|report| report.code() != orisu_diagnostics::Code::UnknownUtility || !report.fixes().is_empty())
            .collect();
        let diagnostics = orisu_diagnostics::resolve(reports, &text, &name);
        if fix && diagnostics.iter().any(|d| !d.fixes.is_empty()) {
            let fixed = orisu_diagnostics::apply_fixes(&text, &diagnostics);
            std::fs::write(file, fixed).map_err(|e| format!("Could not write `{}`: {e}", file.display()))?;
        }
        out.extend(diagnostics);
    }
    Ok(out)
}
