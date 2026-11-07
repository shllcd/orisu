use serde::Deserialize;
use serde::de::DeserializeOwned;
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

#[derive(Debug, Clone)]
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

#[derive(Debug, Clone)]
pub struct Settings {
    pub class_attributes: Vec<String>,
    pub class_functions: Vec<String>,
    pub root_font_size: f64,
    pub lint: Lint,
    /// CSS entry points, relative to the workspace; found automatically when empty.
    pub config_file: Vec<PathBuf>,
}

/// Class functions linted besides those the mangler renames in: variant libraries, whose class lists it cannot rename.
const VARIANT_FUNCTIONS: &[&str] = &["cva", "tv", "tw"];

impl Default for Settings {
    fn default() -> Self {
        let strings = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        Self {
            class_attributes: strings(orisu_analyze::DEFAULT_CLASS_ATTRIBUTES),
            class_functions: orisu_analyze::DEFAULT_CLASS_FUNCTIONS.iter().chain(VARIANT_FUNCTIONS).map(|s| (*s).to_owned()).collect(),
            root_font_size: 16.0,
            lint: Lint::default(),
            config_file: Vec::new(),
        }
    }
}

impl Settings {
    /// What `orisu check` reports: the defaults, plus unknown variants and misspelled utilities.
    pub fn check() -> Self {
        let mut settings = Self::default();
        settings.lint.unknown_classes = Level::Error;
        settings
    }

    /// Reads `{ "orisu": … }`, Tailwind IntelliSense's `{ "tailwindCSS": … }` or the bare object. A field of the wrong type keeps its
    /// default and is described in the returned errors.
    pub fn from_value(value: &Value) -> (Self, Vec<String>) {
        let value = value.get("orisu").or_else(|| value.get("tailwindCSS")).unwrap_or(value);
        let mut settings = Self::default();
        let mut errors = Vec::new();
        read(value, "classAttributes", &mut settings.class_attributes, &mut errors);
        read(value, "classFunctions", &mut settings.class_functions, &mut errors);
        read(value, "rootFontSize", &mut settings.root_font_size, &mut errors);
        if let Some(lint) = value.get("lint") {
            let target = &mut settings.lint;
            read(lint, "cssConflict", &mut target.css_conflict, &mut errors);
            read(lint, "suggestCanonicalClasses", &mut target.suggest_canonical_classes, &mut errors);
            read(lint, "unknownClasses", &mut target.unknown_classes, &mut errors);
            read(lint, "invalidThemeKey", &mut target.invalid_theme_key, &mut errors);
        }
        let config = value.get("configFile").or_else(|| value.pointer("/experimental/configFile"));
        if let Some(config) = config {
            settings.config_file = match config {
                Value::String(path) => vec![PathBuf::from(path)],
                Value::Array(paths) => paths
                    .iter()
                    .filter_map(|path| {
                        let path = path.as_str();
                        if path.is_none() {
                            errors.push("`configFile`: expected an array of strings".to_owned());
                        }
                        path.map(PathBuf::from)
                    })
                    .collect(),
                Value::Object(map) => map.keys().map(PathBuf::from).collect(),
                _ => {
                    errors.push("`configFile`: expected a string, an array of strings or an object".to_owned());
                    Vec::new()
                }
            };
        }
        (settings, errors)
    }
}

fn read<T: DeserializeOwned>(value: &Value, key: &str, slot: &mut T, errors: &mut Vec<String>) {
    match value.get(key).map(T::deserialize) {
        Some(Ok(read)) => *slot = read,
        Some(Err(error)) => errors.push(format!("`{key}`: {error}")),
        None => {}
    }
}
