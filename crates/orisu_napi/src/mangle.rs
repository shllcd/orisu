use crate::{Compiler, MergeTable};
use napi::{Error, Result};
use napi_derive::napi;
use orisu_analyze::Fragment;
use orisu_merge::Merger;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[napi(object)]
#[derive(Clone, Default)]
pub struct MangleOptions {
    /// Starts every short name.
    pub prefix: Option<String>,
    /// Classes that keep their names.
    pub keep: Option<Vec<String>>,
    /// Functions whose arguments are class lists, besides `cn`, `clsx` and the like.
    pub class_functions: Option<Vec<String>>,
    /// Attributes and properties whose values are class lists, besides `class` and `className`.
    pub class_attributes: Option<Vec<String>>,
    pub variant_groups: Option<bool>,
}

impl MangleOptions {
    fn core(&self) -> Result<orisu_mangle::Options> {
        let prefix = self.prefix.clone().unwrap_or_default();
        if !orisu_mangle::names::valid_prefix(&prefix) {
            return Err(Error::from_reason(format!(
                "`mangle.prefix` must be a CSS identifier start of letters, digits, `_` and `-`, got `{prefix}`"
            )));
        }
        Ok(orisu_mangle::Options {
            prefix,
            keep: self.keep.clone().unwrap_or_default(),
            positions: orisu_analyze::Options {
                functions: self.class_functions.clone().unwrap_or_default(),
                attributes: self.class_attributes.clone().unwrap_or_default(),
            },
            variant_groups: self.variant_groups.unwrap_or(false),
        })
    }
}

#[napi(object)]
pub struct MangleInput {
    /// Every scanned file.
    pub files: Vec<String>,
    /// The stylesheet that is renamed.
    pub entry: String,
    pub candidates: Vec<String>,
    /// The stylesheet built from `candidates`.
    pub css: String,
}

#[napi(object)]
pub struct ClassFragment {
    /// `prefix`, `suffix` or `infix`: where runtime text joins the visible part.
    pub kind: String,
    pub text: String,
}

#[napi(object)]
pub struct MangledModule {
    pub code: Option<String>,
    pub map: Option<String>,
    pub pinned: Vec<String>,
    pub fragments: Vec<ClassFragment>,
    pub name_merger: Option<String>,
}

#[napi(object)]
pub struct FilledCode {
    pub code: String,
    pub map: String,
}

#[napi(object)]
pub struct NameTable {
    pub prefix: String,
    pub groups: Vec<String>,
    pub opaque: u32,
    pub runs: Vec<RunEntry>,
    pub holes: Vec<f64>,
}

#[napi(object)]
pub struct RunEntry {
    pub group: u32,
    pub shapes: Vec<u32>,
}

fn fragment_to_js(fragment: Fragment) -> ClassFragment {
    let (kind, text) = match fragment {
        Fragment::Prefix(text) => ("prefix", text),
        Fragment::Suffix(text) => ("suffix", text),
        Fragment::Infix(text) => ("infix", text),
    };
    ClassFragment { kind: kind.to_owned(), text }
}

fn fragment_from_js(fragment: &ClassFragment) -> Fragment {
    let text = fragment.text.clone();
    match fragment.kind.as_str() {
        "prefix" => Fragment::Prefix(text),
        "suffix" => Fragment::Suffix(text),
        _ => Fragment::Infix(text),
    }
}

/// Rewrites the class positions of a module into placeholders that `ManglePlan.fill` resolves.
#[napi]
pub fn mangle_module(code: String, path: String, options: Option<MangleOptions>) -> Result<MangledModule> {
    let options = options.unwrap_or_default().core()?;
    let module = orisu_mangle::module(&code, Path::new(&path), &options).map_err(Error::from_reason)?;
    let (code, map) = module.code.map_or((None, None), |(code, map)| (Some(code), Some(map)));
    Ok(MangledModule {
        code,
        map,
        pinned: module.pinned,
        fragments: module.fragments.into_iter().map(fragment_to_js).collect(),
        name_merger: module.name_merger,
    })
}

#[napi]
impl Compiler {
    /// Picks the classes to rename and their short names.
    #[napi]
    pub fn mangle(&mut self, input: MangleInput, options: Option<MangleOptions>) -> Result<ManglePlan> {
        let options = options.unwrap_or_default().core()?;
        let files: Vec<PathBuf> = input.files.into_iter().map(PathBuf::from).collect();
        self.with_mut(|compiler| {
            let inline = compiler.inline_candidates.clone();
            let core_input = orisu_mangle::Input {
                files: &files,
                entry: Path::new(&input.entry),
                candidates: &input.candidates,
                css: &input.css,
                inline: &inline,
            };
            let prefix = compiler.design_system.theme.prefix().map(str::to_owned);
            let plan = orisu_mangle::plan(&mut Merger::new(compiler.session()), &core_input, &options);
            Ok(ManglePlan { inner: plan, prefix })
        })
    }
}

#[napi]
pub struct ManglePlan {
    inner: orisu_mangle::Plan,
    prefix: Option<String>,
}

fn invalid(what: &str) -> Error {
    Error::from_reason(format!("the saved mangle plan has an invalid `{what}`"))
}

fn strings(value: &Value, what: &str) -> Result<Vec<String>> {
    value.as_array().ok_or_else(|| invalid(what))?.iter().map(|v| v.as_str().map(str::to_owned).ok_or_else(|| invalid(what))).collect()
}

fn numbers(value: &Value, what: &str) -> Result<Vec<u64>> {
    value.as_array().ok_or_else(|| invalid(what))?.iter().map(|v| v.as_u64().ok_or_else(|| invalid(what))).collect()
}

#[napi]
impl ManglePlan {
    /// `css` with the renamed classes of its selectors replaced by their short names.
    #[napi]
    pub fn rename_css(&self, css: String) -> String {
        self.inner.rename_css(&css)
    }

    /// `code` with its placeholders resolved; `null` when it has none.
    #[napi]
    pub fn fill(&self, code: String, name: String) -> Option<FilledCode> {
        self.inner.fill(&code, &name).map(|(code, map)| FilledCode { code, map })
    }

    /// The error of a built module that uses renamed classes where they cannot be renamed.
    #[napi]
    pub fn check_module(&self, file: String, module: MangledModule) -> Option<String> {
        let module = orisu_mangle::Module {
            code: None,
            pinned: module.pinned,
            fragments: module.fragments.iter().map(fragment_from_js).collect(),
            name_merger: module.name_merger,
        };
        self.inner.check_module(&file, &module)
    }

    /// The error of a stylesheet outside the build whose classes the plan renames or reuses.
    #[napi]
    pub fn check_css(&self, file: String, css: String) -> Option<String> {
        self.inner.check_css(&file, &css)
    }

    /// Renamed classes and their short names, in naming order.
    #[napi(getter)]
    pub fn names(&self) -> Vec<Vec<String>> {
        self.inner.names.iter().map(|(class, short)| vec![class.clone(), short.clone()]).collect()
    }

    /// The merge table of the classes that keep their names, with the short names of the others.
    #[napi]
    pub fn merge_table(&self) -> MergeTable {
        let table = &self.inner.table;
        let names = &self.inner.name_table;
        MergeTable {
            prefix: self.prefix.clone(),
            keys: table.entries.keys().cloned().collect(),
            values: table.entries.values().copied().collect(),
            erases: table.erases.clone(),
            names: (!self.inner.names.is_empty()).then(|| NameTable {
                prefix: names.prefix.clone(),
                groups: names.groups.clone(),
                opaque: names.opaque,
                runs: names.runs.iter().map(|(group, shapes)| RunEntry { group: *group, shapes: shapes.clone() }).collect(),
                holes: names.holes.iter().map(|&k| k as f64).collect(),
            }),
        }
    }

    /// The plan as JSON, for a build in another process to read with `ManglePlan.fromJson`.
    #[napi]
    pub fn to_json(&self) -> String {
        let plan = &self.inner;
        let names = &plan.name_table;
        json!({
            "prefix": self.prefix,
            "names": plan.names.iter().flat_map(|(class, short)| [class, short]).collect::<Vec<_>>(),
            "keys": plan.table.entries.keys().collect::<Vec<_>>(),
            "values": plan.table.entries.values().collect::<Vec<_>>(),
            "erases": plan.table.erases,
            "namePrefix": names.prefix,
            "groups": names.groups,
            "opaque": names.opaque,
            "runs": names.runs,
            "holes": names.holes,
        })
        .to_string()
    }

    #[napi(factory)]
    pub fn from_json(json: String) -> Result<ManglePlan> {
        let value: Value = serde_json::from_str(&json).map_err(|e| Error::from_reason(e.to_string()))?;
        let pairs = strings(&value["names"], "names")?;
        let names: indexmap::IndexMap<String, String, rustc_hash::FxBuildHasher> =
            pairs.chunks(2).map(|pair| (pair[0].clone(), pair.get(1).cloned().unwrap_or_default())).collect();
        let keys = strings(&value["keys"], "keys")?;
        let values = numbers(&value["values"], "values")?;
        let erases = value["erases"]
            .as_array()
            .ok_or_else(|| invalid("erases"))?
            .iter()
            .map(|list| numbers(list, "erases").map(|list| list.into_iter().map(|n| n as u32).collect()))
            .collect::<Result<Vec<Vec<u32>>>>()?;
        let runs = value["runs"]
            .as_array()
            .ok_or_else(|| invalid("runs"))?
            .iter()
            .map(|run| {
                let group = run[0].as_u64().ok_or_else(|| invalid("runs"))? as u32;
                let shapes = numbers(&run[1], "runs")?.into_iter().map(|n| n as u32).collect();
                Ok((group, shapes))
            })
            .collect::<Result<Vec<_>>>()?;
        let shorts = names.values().cloned().collect();
        let inner = orisu_mangle::Plan {
            names,
            shorts,
            table: orisu_merge::MergeTable { erases, entries: keys.into_iter().zip(values.into_iter().map(|n| n as u32)).collect() },
            name_table: orisu_mangle::NameTable {
                prefix: value["namePrefix"].as_str().unwrap_or_default().to_owned(),
                groups: strings(&value["groups"], "groups")?,
                opaque: value["opaque"].as_u64().ok_or_else(|| invalid("opaque"))? as u32,
                runs,
                holes: numbers(&value["holes"], "holes")?,
            },
        };
        Ok(ManglePlan { inner, prefix: value["prefix"].as_str().map(str::to_owned) })
    }
}
