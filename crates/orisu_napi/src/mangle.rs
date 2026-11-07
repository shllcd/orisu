use crate::{Compiler, MergeTable};
use napi::{Error, Result};
use napi_derive::napi;
use orisu_analyze::Fragment;
use orisu_merge::Merger;
use serde_json::Value;
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
    /// Attributes and properties whose values are class lists, besides `class`, `className`, `*ClassName` and framework forms like `:class` and `[ngClass]`.
    pub class_attributes: Option<Vec<String>>,
    pub variant_groups: Option<bool>,
}

impl MangleOptions {
    fn core(&self) -> Result<orisu_mangle::Options> {
        orisu_mangle::Options::new(
            self.prefix.clone().unwrap_or_default(),
            self.keep.clone().unwrap_or_default(),
            orisu_analyze::Options {
                functions: self.class_functions.clone().unwrap_or_default(),
                attributes: self.class_attributes.clone().unwrap_or_default(),
            },
            self.variant_groups.unwrap_or(false),
        )
        .map_err(Error::from_reason)
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

fn fragment_from_js(fragment: &ClassFragment) -> Result<Fragment> {
    let text = fragment.text.clone();
    Ok(match fragment.kind.as_str() {
        "prefix" => Fragment::Prefix(text),
        "suffix" => Fragment::Suffix(text),
        "infix" => Fragment::Infix(text),
        kind => return Err(Error::from_reason(format!("unknown class fragment kind `{kind}`"))),
    })
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

/// `code` with its placeholders turned back into the classes they hold, for a build without a plan; `null` when it has none.
#[napi]
pub fn fill_placeholders(code: String, name: String) -> Option<FilledCode> {
    orisu_mangle::fill_placeholders(&code, &name).map(|(code, map)| FilledCode { code, map })
}

#[napi]
impl Compiler {
    /// Picks the classes to rename and their short names.
    #[napi]
    pub fn mangle(&mut self, input: MangleInput, options: Option<MangleOptions>) -> Result<ManglePlan> {
        let options = options.unwrap_or_default().core()?;
        let files: Vec<PathBuf> = input.files.into_iter().map(PathBuf::from).collect();
        self.0.with_mut(|compiler| {
            let inline = compiler.inline_candidates.clone();
            let core_input = orisu_mangle::Input {
                files: &files,
                entry: Path::new(&input.entry),
                candidates: &input.candidates,
                css: &input.css,
                inline: &inline,
            };
            let prefix = compiler.design_system.theme.prefix().map(str::to_owned);
            let plan = orisu_mangle::plan(&mut Merger::new(compiler.session()), &core_input, &options).map_err(Error::from_reason)?;
            Ok(ManglePlan { inner: plan, prefix })
        })
    }
}

#[napi]
pub struct ManglePlan {
    inner: orisu_mangle::Plan,
    prefix: Option<String>,
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
    pub fn check_module(&self, file: String, module: MangledModule) -> Result<Option<String>> {
        let module = orisu_mangle::Module {
            code: None,
            pinned: module.pinned,
            fragments: module.fragments.iter().map(fragment_from_js).collect::<Result<_>>()?,
            name_merger: module.name_merger,
        };
        Ok(self.inner.check_module(&file, &module))
    }

    /// The error of a stylesheet outside the build whose classes the plan renames or reuses.
    #[napi]
    pub fn check_css(&self, file: String, css: String) -> Option<String> {
        self.inner.check_css(&file, &css)
    }

    /// Renamed classes and their short names, in naming order.
    #[napi(getter)]
    pub fn names(&self) -> Vec<Vec<String>> {
        self.inner.names().iter().map(|(class, short)| vec![class.clone(), short.clone()]).collect()
    }

    /// The merge table of the classes that keep their names, with the short names of the others.
    #[napi]
    pub fn merge_table(&self) -> MergeTable {
        let table = self.inner.table();
        let names = self.inner.name_table();
        MergeTable {
            prefix: self.prefix.clone(),
            keys: table.keys.clone(),
            values: table.values.clone(),
            erases: table.erases.clone(),
            names: (!self.inner.names().is_empty()).then(|| NameTable {
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
        let mut out = self.inner.to_json();
        out.insert("prefix".to_owned(), self.prefix.clone().into());
        Value::Object(out).to_string()
    }

    #[napi(factory)]
    pub fn from_json(json: String) -> Result<ManglePlan> {
        let value: Value = serde_json::from_str(&json).map_err(|e| Error::from_reason(e.to_string()))?;
        let inner = orisu_mangle::Plan::from_json(&value).map_err(Error::from_reason)?;
        Ok(ManglePlan { inner, prefix: value["prefix"].as_str().map(str::to_owned) })
    }
}
