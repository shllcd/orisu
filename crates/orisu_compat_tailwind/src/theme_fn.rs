// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use indexmap::IndexMap;
use orisu_allocator::AllocExt;
use orisu_compiler::design_system::{DesignSystem, Session};
use orisu_parser::escape::unescape;
use orisu_parser::segment::segment;
use orisu_theme::{UsedVars, opts};
use rustc_hash::FxBuildHasher;
use serde::de::{Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::sync::LazyLock;

type Map = IndexMap<String, Value, FxBuildHasher>;
type ByKey<'k, V> = IndexMap<Option<&'k str>, V, FxBuildHasher>;

/// A JavaScript value as the v3 theme sees it; `Opaque` stands for the config's `__BARE_VALUE__` functions.
#[derive(Debug, Clone)]
enum Value {
    Null,
    Opaque,
    Num(u8),
    Str(String),
    Arr(Vec<Value>),
    Obj(Map),
}

impl Value {
    fn num(&self) -> u8 {
        if let Value::Num(n) = self { *n } else { 0 }
    }

    fn or(self, other: Value) -> Value {
        if matches!(self, Value::Null) { other } else { self }
    }

    fn own(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(map) => map.get(key),
            Value::Arr(items) => key.parse::<usize>().ok().filter(|i| i.to_string() == key).and_then(|i| items.get(i)),
            _ => None,
        }
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ValueVisitor;

        impl<'de> Visitor<'de> for ValueVisitor {
            type Value = Value;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a theme value")
            }

            fn visit_bool<E>(self, _: bool) -> Result<Value, E> {
                Ok(Value::Opaque)
            }

            fn visit_unit<E>(self) -> Result<Value, E> {
                Ok(Value::Null)
            }

            fn visit_str<E>(self, value: &str) -> Result<Value, E> {
                Ok(Value::Str(value.to_owned()))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Value::Arr(items))
            }

            fn visit_map<A: MapAccess<'de>>(self, mut access: A) -> Result<Value, A::Error> {
                let mut map = Map::default();
                while let Some((key, value)) = access.next_entry()? {
                    map.insert(key, value);
                }
                Ok(Value::Obj(map))
            }
        }

        deserializer.deserialize_any(ValueVisitor)
    }
}

/// Tailwind's `default-theme.ts` with each `({ theme }) => …` entry recorded as `{"theme": key}` or
/// `{"spread": [key | {…}]}`, in the order its parts are spread.
static DEFAULT_THEME: LazyLock<Map> = LazyLock::new(|| match serde_json::from_str(include_str!("../assets/default-theme.json")) {
    Ok(Value::Obj(map)) => map,
    _ => unreachable!("default-theme.json is an object"),
});

const EXTEND: &[(&str, &str)] = &[
    ("fontSize", "text"),
    ("boxShadow", "shadow"),
    ("animation", "animate"),
    ("aspectRatio", "aspect"),
    ("borderRadius", "radius"),
    ("screens", "breakpoint"),
    ("letterSpacing", "tracking"),
    ("lineHeight", "leading"),
    ("maxWidth", "container"),
];

const DEFAULT_TRANSITIONS: &[(&str, &str)] =
    &[("transitionDuration", "--default-transition-duration"), ("transitionTimingFunction", "--default-transition-timing-function")];

const SPECIAL_DEFAULT_KEYS: &[(&str, &str)] = &[
    ("borderWidth", "border-width"),
    ("outlineWidth", "outline-width"),
    ("ringColor", "ring-color"),
    ("ringWidth", "ring-width"),
    ("transitionDuration", "transition-duration"),
    ("transitionTimingFunction", "transition-timing-function"),
];

const OLD_TO_NEW_NAMESPACE: &[(&str, &str)] = &[
    ("animation", "animate"),
    ("aspectRatio", "aspect"),
    ("borderRadius", "radius"),
    ("boxShadow", "shadow"),
    ("colors", "color"),
    ("containers", "container"),
    ("fontFamily", "font"),
    ("fontSize", "text"),
    ("letterSpacing", "tracking"),
    ("lineHeight", "leading"),
    ("maxWidth", "container"),
    ("screens", "breakpoint"),
    ("transitionTimingFunction", "ease"),
];

const CSS_VALUES: &str = "__CSS_VALUES__";

fn lookup(table: &[(&str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Resolves a v3 dot path such as `colors.red.500 / 50%` against the CSS theme and Tailwind's default
/// JS config, as `@config`-less Tailwind does once a stylesheet uses `theme()` with one.
pub fn resolve<'a>(ds: &Session<'_, 'a>, path: &str) -> Option<&'a str> {
    let resolver = Resolver { ds: ds.ds, used: UsedVars::default() };
    let (path, modifier) = match path.rfind('/') {
        Some(i) => (path[..i].trim(), Some(path[i + 1..].trim()).filter(|m| !m.is_empty())),
        None => (path, None),
    };
    let mut value = resolver.resolve(to_key_path(path));
    if let (Some(modifier), Value::Str(s)) = (modifier, &mut value) {
        *s = ds.cx().with_alpha(ds.alloc.str(s), ds.alloc.str(modifier)).to_owned();
    }
    let value = match value {
        Value::Arr(items) if items.len() == 2 => items.into_iter().next()?,
        Value::Arr(items) => Value::Str(join(&items, ", ")),
        Value::Obj(mut map) if map.contains_key("DEFAULT") => map.swap_remove("DEFAULT")?,
        value => value,
    };
    match value {
        Value::Str(value) => Some(ds.alloc.str(&value)),
        _ => None,
    }
}

fn join(items: &[Value], separator: &str) -> String {
    let parts: Vec<String> = items
        .iter()
        .map(|item| match item {
            Value::Null | Value::Opaque => String::new(),
            Value::Num(n) => n.to_string(),
            Value::Str(s) => s.clone(),
            Value::Arr(items) => join(items, ","),
            Value::Obj(_) => "[object Object]".to_owned(),
        })
        .collect();
    parts.join(separator)
}

struct Resolver<'d, 'a> {
    ds: &'d DesignSystem<'a>,
    used: UsedVars,
}

impl Resolver<'_, '_> {
    fn theme(&self, path: &str, default: Value) -> Value {
        self.resolve(to_key_path(path)).or(default)
    }

    fn resolve(&self, keypath: Vec<String>) -> Value {
        let (mut css, options) = self.read_from_css(&keypath);
        let mut config = self.get(None, keypath).unwrap_or(Value::Null);
        if let Value::Str(s) = &mut config {
            *s = s.replacen("<alpha-value>", "1", 1);
        }

        if let Value::Str(_) = css {
            if options.num() & opts::DEFAULT != 0 {
                return config.or(css);
            }
            return css;
        }

        if let Value::Obj(config) = &config {
            let mut copy = config.clone();
            let config_options = config.get(CSS_VALUES);
            if let (Value::Null, Some(Value::Obj(keys))) = (&css, config_options) {
                let mut local = Map::default();
                for key in keys.keys() {
                    if let Some(value) = config.get(key) {
                        local.insert(key.clone(), value.clone());
                    }
                    copy.shift_remove(key);
                }
                css = Value::Obj(local);
            }
            let entries: Vec<(String, Value)> = match css {
                Value::Obj(map) => map.into_iter().collect(),
                Value::Arr(items) => items.into_iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
                _ => Vec::new(),
            };
            let mut copy = Value::Obj(copy);
            for (key, value) in entries {
                if key == CSS_VALUES {
                    continue;
                }
                let is_default = config_options.and_then(|o| o.own(&key)).is_some_and(|o| o.num() & opts::DEFAULT != 0);
                if is_default && get_in(&copy, key.split('-').map(str::to_owned).collect()).is_some() {
                    continue;
                }
                if let Value::Obj(copy) = &mut copy {
                    copy.insert(unescape(&key).into_owned(), value);
                }
            }
            return copy;
        }

        if let (Value::Arr(css), Value::Arr(options), Value::Arr(config)) = (&css, &options, &config) {
            let mut base = css.first().cloned().unwrap_or(Value::Null);
            let mut extra = css.get(1).cloned().unwrap_or(Value::Null);
            if options.first().is_some_and(|o| o.num() & opts::DEFAULT != 0) {
                base = config.first().cloned().unwrap_or(Value::Null).or(base);
            }
            if let Value::Obj(extra) = &mut extra {
                for (key, value) in extra.iter_mut() {
                    let is_default = options.get(1).and_then(|o| o.own(key)).is_some_and(|o| o.num() & opts::DEFAULT != 0);
                    if let Some(configured) = config.get(1).and_then(|c| c.own(key)).filter(|_| is_default) {
                        *value = configured.clone().or(value.clone());
                    }
                }
            }
            return Value::Arr(vec![base, extra]);
        }

        if let Value::Obj(map) = &css
            && let Some(default) = map.get("DEFAULT")
        {
            return default.clone();
        }
        css.or(config)
    }

    fn options(&self, key: &str) -> u8 {
        self.ds.theme.get_options(key, &self.used)
    }

    fn read_from_css(&self, path: &[String]) -> (Value, Value) {
        let theme_key = key_path_to_css_property(path).unwrap_or_else(|| "null".to_owned());
        let namespace = format!("--{theme_key}");
        let entries = self.ds.theme.namespace(&namespace);
        if entries.is_empty() {
            return (Value::Null, Value::Num(opts::NONE));
        }

        let mut map: ByKey<Value> = IndexMap::default();
        let mut options: ByKey<Value> = IndexMap::default();
        let mut nested: ByKey<Vec<(String, &str, u8)>> = IndexMap::default();
        for (key, value) in entries {
            let Some(at) = key.and_then(|k| k.find("--")) else {
                map.insert(key, Value::Str(value.to_owned()));
                let full = match key {
                    Some(key) => format!("{namespace}-{key}"),
                    None => namespace.clone(),
                };
                options.insert(key, Value::Num(self.options(&full)));
                continue;
            };
            let key = key.unwrap_or_default();
            let main = Some(&key[..at]).filter(|m| !m.is_empty());
            let sub = camelize(&key[at + 2..]);
            nested.entry(main).or_default().push((sub, value, self.options(&format!("{namespace}{key}"))));
        }

        let base_options = self.options(&namespace);
        for (key, extra) in nested {
            let Some(Value::Str(value)) = map.get(&key) else {
                continue;
            };
            let mut values = Map::default();
            let mut extra_options = Map::default();
            for (sub, sub_value, sub_options) in extra {
                values.insert(sub.clone(), Value::Str(sub_value.to_owned()));
                extra_options.insert(sub, Value::Num(sub_options));
            }
            let tuple = Value::Arr(vec![Value::Str(value.clone()), Value::Obj(values)]);
            map.insert(key, tuple);
            options.insert(key, Value::Arr(vec![Value::Num(base_options), Value::Obj(extra_options)]));
        }

        let mut obj = Map::default();
        for (key, value) in map {
            obj.insert(key.unwrap_or("DEFAULT").to_owned(), value);
        }
        let mut options_obj = Map::default();
        for (key, value) in options {
            options_obj.insert(key.unwrap_or("DEFAULT").to_owned(), value);
        }

        if path.last().is_some_and(|p| p == "DEFAULT") || (obj.len() == 1 && obj.contains_key("DEFAULT")) {
            let value = obj.swap_remove("DEFAULT").unwrap_or(Value::Null);
            return (value, options_obj.swap_remove("DEFAULT").unwrap_or(Value::Num(opts::NONE)));
        }
        obj.insert(CSS_VALUES.to_owned(), Value::Obj(options_obj.clone()));
        (Value::Obj(obj), Value::Obj(options_obj))
    }

    fn get(&self, root: Option<Value>, mut path: Vec<String>) -> Option<Value> {
        let mut current = root;
        for i in 0..path.len() {
            let found = match &current {
                None => self.config(&path[i]),
                Some(value) => value.own(&path[i]).cloned(),
            };
            match found {
                Some(value) => current = Some(value),
                None if i + 1 == path.len() => return None,
                None => path[i + 1] = format!("{}-{}", path[i], path[i + 1]),
            }
        }
        current
    }

    fn config(&self, key: &str) -> Option<Value> {
        let extension = lookup(EXTEND, key).map(|namespace| spread(self.theme(namespace, Value::Obj(Map::default())))).or_else(|| {
            lookup(DEFAULT_TRANSITIONS, key).map(|variable| {
                let default = self.ds.theme.get(&[variable]).map_or(Value::Null, |v| Value::Str(v.to_owned()));
                Map::from_iter([("DEFAULT".to_owned(), default)])
            })
        });
        let base = if key == "colors" { Some(self.theme("color", Value::Obj(Map::default()))) } else { self.default_entry(key) };
        let mut value = match (base, extension) {
            (None, None) => return None,
            (base, Some(extension)) => {
                let mut out = Map::default();
                for source in [base.unwrap_or(Value::Null), Value::Obj(extension)] {
                    if let Value::Obj(source) = source {
                        deep_merge(&mut out, source);
                    }
                }
                Value::Obj(out)
            }
            (Some(base), None) => base,
        };
        if key == "screens"
            && let Value::Obj(screens) = &mut value
        {
            for screen in screens.values_mut() {
                if let Value::Obj(s) = screen
                    && !s.contains_key("raw")
                    && !s.contains_key("max")
                    && let Some(min) = s.get("min")
                {
                    *screen = min.clone();
                }
            }
        }
        Some(value)
    }

    fn default_entry(&self, key: &str) -> Option<Value> {
        let Value::Obj(entry) = DEFAULT_THEME.get(key)? else {
            return None;
        };
        if let Some(value) = entry.get("value") {
            return Some(value.clone());
        }
        if let Some(Value::Str(path)) = entry.get("theme") {
            return Some(self.theme(path, Value::Null));
        }
        let Some(Value::Arr(parts)) = entry.get("spread") else {
            return None;
        };
        let mut out = Map::default();
        for part in parts {
            let part = match part {
                Value::Str(path) => spread(self.theme(path, Value::Null)),
                Value::Obj(map) => map.clone(),
                _ => continue,
            };
            out.extend(part);
        }
        Some(Value::Obj(out))
    }
}

fn get_in(value: &Value, mut path: Vec<String>) -> Option<Value> {
    let mut current = value;
    for i in 0..path.len() {
        match current.own(&path[i]) {
            Some(next) => current = next,
            None if i + 1 == path.len() => return None,
            None => path[i + 1] = format!("{}-{}", path[i], path[i + 1]),
        }
    }
    Some(current.clone())
}

fn spread(value: Value) -> Map {
    match value {
        Value::Obj(map) => map,
        Value::Arr(items) => items.into_iter().enumerate().map(|(i, v)| (i.to_string(), v)).collect(),
        Value::Str(s) => s.chars().enumerate().map(|(i, c)| (i.to_string(), Value::Str(c.to_string()))).collect(),
        _ => Map::default(),
    }
}

fn deep_merge(out: &mut Map, source: Map) {
    for (key, b) in source {
        let a = out.get(&key);
        if let Some(merged) = merge_extension(a, &b) {
            out.insert(key, merged);
            continue;
        }
        match (out.get_mut(&key), b) {
            (Some(Value::Obj(a)), Value::Obj(b)) => {
                let mut merged = Map::default();
                deep_merge(&mut merged, std::mem::take(a));
                deep_merge(&mut merged, b);
                *a = merged;
            }
            (_, b) => {
                out.insert(key, b);
            }
        }
    }
}

fn merge_extension(a: Option<&Value>, b: &Value) -> Option<Value> {
    if let Some(Value::Arr(items)) = a
        && let Some(Value::Obj(_)) = items.first()
    {
        let mut items = items.clone();
        match b {
            Value::Arr(more) => items.extend(more.iter().cloned()),
            b => items.push(b.clone()),
        }
        return Some(Value::Arr(items));
    }
    if let Value::Arr(more) = b {
        if let (Some(Value::Obj(_)), Some(a @ Value::Obj(_))) = (more.first(), a) {
            return Some(Value::Arr(std::iter::once(a.clone()).chain(more.iter().cloned()).collect()));
        }
        return Some(b.clone());
    }
    None
}

/// `a.b[c.d]` → `["a", "b", "c.d"]`.
fn to_key_path(path: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for part in segment(path, b'.') {
        if !part.contains('[') {
            keys.push(part.to_owned());
            continue;
        }
        let mut at = 0;
        while let Some(open) = part[at..].find('[').map(|i| at + i)
            && let Some(close) = part[open..].find(']').map(|i| open + i)
        {
            if open > at {
                keys.push(part[at..open].to_owned());
            }
            keys.push(part[open + 1..close].to_owned());
            at = close + 1;
        }
        if at < part.len() {
            keys.push(part[at..].to_owned());
        }
    }
    keys
}

/// `["fontSize", "xs", "1", "lineHeight"]` → `text-xs--line-height`, or `None` when it has no CSS variable.
fn key_path_to_css_property(path: &[String]) -> Option<String> {
    let first = path.first()?;
    if let Some(special) = lookup(SPECIAL_DEFAULT_KEYS, first)
        && path.get(1).is_some_and(|p| p == "DEFAULT")
    {
        return Some(format!("default-{special}"));
    }
    if first == "container" {
        return None;
    }
    let valid = |part: &String| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_%/.".contains(&b));
    if !path.iter().all(valid) {
        return None;
    }
    let last = path.len() - 1;
    let mut parts = Vec::with_capacity(path.len());
    for (i, part) in path.iter().enumerate() {
        let part = if i == 0 { lookup(OLD_TO_NEW_NAMESPACE, part).unwrap_or(part) } else { part };
        if part == "DEFAULT" && i == last {
            continue;
        }
        let part = if part == "1" && i != last { "" } else { part };
        let part = part.replace('.', "_");
        parts.push(if i == 0 || part.starts_with('-') || part == "lineHeight" { kebab(&part) } else { part });
    }
    Some(parts.join("-"))
}

fn kebab(part: &str) -> String {
    let mut out = String::with_capacity(part.len() + 4);
    let mut chars = part.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c.is_ascii_lowercase()
            && let Some(next) = chars.next_if(char::is_ascii_uppercase)
        {
            out.push('-');
            out.push(next.to_ascii_lowercase());
        }
    }
    out
}

fn camelize(sub: &str) -> String {
    let mut out = String::with_capacity(sub.len());
    let mut chars = sub.chars().peekable();
    while let Some(c) = chars.next() {
        match chars.peek() {
            Some(next) if c == '-' && next.is_ascii_lowercase() => {
                out.push(next.to_ascii_uppercase());
                chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}
