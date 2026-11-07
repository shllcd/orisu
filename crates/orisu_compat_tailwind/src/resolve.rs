//! Node-style resolution of CSS `@import`s, as `@tailwindcss/node` does it: relative files, then packages in `node_modules`
//! and `NODE_PATH` through `exports` (style conditions, in the package's order), `style`, `main` and `index.css`.

use indexmap::IndexMap;
use serde::Deserialize;
use std::path::{Component, Path, PathBuf};

const STYLE_CONDITIONS: &[&str] = &["style", "development", "production", "import", "default"];

#[derive(Deserialize)]
#[serde(untagged)]
enum Target {
    Path(String),
    List(Vec<Target>),
    Conditions(IndexMap<String, Target>),
    Other(serde::de::IgnoredAny),
}

#[derive(Deserialize)]
struct Manifest {
    exports: Option<Target>,
    style: Option<Target>,
    main: Option<Target>,
}

/// The absolute, normalized path `id` resolves to from the directory `base`.
pub fn resolve_css(id: &str, base: &Path) -> Option<PathBuf> {
    if id.starts_with('.') || Path::new(id).is_absolute() {
        return with_css_extension(normalize(&base.join(id)));
    }
    let mut parts = id.split('/');
    let name = match parts.next()? {
        scope if scope.starts_with('@') => format!("{scope}/{}", parts.next()?),
        name => name.to_owned(),
    };
    let subpath = parts.collect::<Vec<_>>().join("/");
    let base = normalize(base);
    let node_path = std::env::var_os("NODE_PATH");
    let global = node_path.iter().flat_map(std::env::split_paths).filter(|dir| !dir.as_os_str().is_empty());
    base.ancestors().map(|dir| dir.join("node_modules")).chain(global).find_map(|root| {
        let dir = normalize(&root.join(&name));
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
    let Some(pkg) = manifest.and_then(|m| serde_json::from_str::<Manifest>(&m).ok()) else {
        return with_css_extension(normalize(&dir.join(subpath)));
    };
    let key = if subpath.is_empty() { ".".to_owned() } else { format!("./{subpath}") };
    if let Some(exports) = &pkg.exports {
        let target = match exports {
            Target::Conditions(map) if map.keys().next().is_some_and(|k| k.starts_with('.')) => {
                map.get(&key).and_then(pick_export).or_else(|| map.get(&format!("{key}.css")).and_then(pick_export))
            }
            sugar => (key == ".").then(|| pick_export(sugar)).flatten(),
        };
        if let Some(target) = target {
            return Some(normalize(&dir.join(target)));
        }
    }
    if !subpath.is_empty() {
        return with_css_extension(normalize(&dir.join(subpath)));
    }
    for field in [&pkg.style, &pkg.main] {
        if let Some(Target::Path(file)) = field
            && file.ends_with(".css")
        {
            return Some(normalize(&dir.join(file)));
        }
    }
    let index = dir.join("index.css");
    index.is_file().then_some(index)
}

/// The first target whose conditions match, walking objects in the package's key order as Node does.
fn pick_export(target: &Target) -> Option<&str> {
    match target {
        Target::Path(target) => Some(target),
        Target::List(items) => items.iter().find_map(pick_export),
        Target::Conditions(map) => {
            map.iter().filter(|(condition, _)| STYLE_CONDITIONS.contains(&condition.as_str())).find_map(|(_, target)| pick_export(target))
        }
        Target::Other(_) => None,
    }
}

/// Makes `path` absolute and resolves `.` and `..` lexically, like Node's `path.resolve`.
fn normalize(path: &Path) -> PathBuf {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}
