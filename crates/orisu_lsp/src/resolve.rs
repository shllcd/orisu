//! Node-style CSS resolution, as in the JS package: relative files, then packages through
//! `exports` (`style` condition), `style` and `main`.

use serde_json::Value;
use std::path::{Path, PathBuf};

const STYLE_CONDITIONS: &[&str] = &["style", "development", "production", "import", "default"];

pub fn resolve_css(id: &str, base: &Path) -> Option<PathBuf> {
    if id.starts_with('.') || Path::new(id).is_absolute() {
        return with_css_extension(base.join(id));
    }
    let mut parts = id.split('/');
    let name = match parts.next()? {
        scope if scope.starts_with('@') => format!("{scope}/{}", parts.next()?),
        name => name.to_owned(),
    };
    let subpath = parts.collect::<Vec<_>>().join("/");
    base.ancestors().find_map(|dir| {
        let dir = dir.join("node_modules").join(&name);
        if dir.is_dir() { resolve_package(&dir, &subpath) } else { None }
    })
}

fn with_css_extension(file: PathBuf) -> Option<PathBuf> {
    if file.is_file() {
        return Some(file);
    }
    let mut css = file.into_os_string();
    css.push(".css");
    let css = PathBuf::from(css);
    css.is_file().then_some(css)
}

fn resolve_package(dir: &Path, subpath: &str) -> Option<PathBuf> {
    let manifest = std::fs::read_to_string(dir.join("package.json")).ok();
    let Some(pkg) = manifest.and_then(|m| serde_json::from_str::<Value>(&m).ok()) else {
        return with_css_extension(dir.join(subpath));
    };
    let key = if subpath.is_empty() { ".".to_owned() } else { format!("./{subpath}") };
    if let Some(exports) = pkg.get("exports") {
        let sugar = match exports {
            Value::Object(map) => !map.keys().next().is_some_and(|k| k.starts_with('.')),
            _ => true,
        };
        let target = if sugar {
            (key == ".").then(|| pick_export(exports)).flatten()
        } else {
            exports.get(&key).and_then(pick_export).or_else(|| exports.get(format!("{key}.css")).and_then(pick_export))
        };
        if let Some(target) = target {
            return Some(dir.join(target));
        }
    }
    if !subpath.is_empty() {
        return with_css_extension(dir.join(subpath));
    }
    for field in ["style", "main"] {
        if let Some(file) = pkg.get(field).and_then(Value::as_str).filter(|f| f.ends_with(".css")) {
            return Some(dir.join(file));
        }
    }
    let index = dir.join("index.css");
    index.is_file().then_some(index)
}

fn pick_export(target: &Value) -> Option<&str> {
    match target {
        Value::String(target) => Some(target),
        Value::Array(items) => items.iter().find_map(pick_export),
        Value::Object(map) => STYLE_CONDITIONS.iter().find_map(|condition| map.get(*condition).and_then(pick_export)),
        _ => None,
    }
}
