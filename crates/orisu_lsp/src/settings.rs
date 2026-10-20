use serde::Deserialize;
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ignore,
    Hint,
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Lint {
    pub css_conflict: Level,
    pub suggest_canonical_classes: Level,
    /// Classes that generate no CSS; off by default, since class lists often mix in plain CSS classes.
    pub unknown_classes: Level,
    pub invalid_theme_key: Level,
}

impl Default for Lint {
    fn default() -> Self {
        Self {
            css_conflict: Level::Warning,
            suggest_canonical_classes: Level::Warning,
            unknown_classes: Level::Ignore,
            invalid_theme_key: Level::Warning,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub class_attributes: Vec<String>,
    pub class_functions: Vec<String>,
    pub root_font_size: f64,
    pub lint: Lint,
    /// CSS entry points, relative to the workspace; found automatically when empty.
    #[serde(skip)]
    pub config_file: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        let strings = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        Self {
            class_attributes: strings(&["class", "className", "ngClass", "[ngClass]", "class:list", ":class", "v-bind:class", "[class]"]),
            class_functions: strings(&["cn", "cva", "cx", "clsx", "classnames", "twMerge", "twJoin", "tv", "tw"]),
            root_font_size: 16.0,
            lint: Lint::default(),
            config_file: Vec::new(),
        }
    }
}

impl Settings {
    /// Reads `{ "orisu": … }`, Tailwind IntelliSense's `{ "tailwindCSS": … }` or the bare object.
    pub fn from_value(value: &Value) -> Self {
        let value = value.get("orisu").or_else(|| value.get("tailwindCSS")).unwrap_or(value);
        let mut settings: Self = serde_json::from_value(value.clone()).unwrap_or_default();
        let config = value.get("configFile").or_else(|| value.pointer("/experimental/configFile"));
        if let Some(config) = config {
            settings.config_file = match config {
                Value::String(path) => vec![PathBuf::from(path)],
                Value::Array(paths) => paths.iter().filter_map(Value::as_str).map(PathBuf::from).collect(),
                Value::Object(map) => map.keys().map(PathBuf::from).collect(),
                _ => Vec::new(),
            };
        }
        settings
    }
}
