use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq)]
pub struct GlobEntry {
    /// Base path of the glob
    pub base: String,

    /// Glob pattern
    pub pattern: String,
}

pub fn hoist_static_glob_parts(entries: &Vec<GlobEntry>, emit_parent_glob: bool) -> Vec<GlobEntry> {
    let mut result = vec![];

    for entry in entries {
        let (static_part, dynamic_part) = split_pattern(&entry.pattern);

        let base: PathBuf = entry.base.clone().into();
        let base = match static_part {
            Some(static_part) => base.join(static_part),
            None => base,
        };

        let Ok(base) = dunce::canonicalize(&base) else {
            continue;
        };

        let pattern = match dynamic_part {
            Some(dynamic_part) => dynamic_part,
            None => {
                if base.is_dir() {
                    "**/*".to_owned()
                } else {
                    "".to_owned()
                }
            }
        };

        if emit_parent_glob && pattern.is_empty() && base.is_file() {
            result.push(GlobEntry {
                // SAFETY: `parent()` will be available because we verify `base` is a file.
                base: base.parent().unwrap().to_string_lossy().to_string(),
                // SAFETY: `file_name()` will be available because we verify `base` is a file.
                pattern: base.file_name().unwrap().to_string_lossy().to_string(),
            });
        }

        result.push(GlobEntry { base: base.to_string_lossy().to_string(), pattern });
    }

    result
}

/// This function attempts to optimize the glob patterns to improve performance. The problem is
/// that if you run the following command:
/// ```sh
/// tailwind --pwd ./project --content "{pages,components}/**/*.js"
/// ```
/// Then the globwalk library will scan every single file and folder in the `./project` folder,
/// then it will check if the file matches the glob pattern and keep it if it does. This is very
/// slow, because if you have vendor folders (like node_modules), then this will take a while...
///
/// Instead, we will optimize the pattern, and move as many directories as possible to the base
/// path. This will allow us to scope the globwalk library to only scan the directories that we
/// care about.
///
/// This means, that the following command:
/// ```sh
/// tailwind --pwd ./project --content "{pages,components}/**/*.js"
/// ```
///
/// Will now conceptually do this instead behind the scenes:
/// ```sh
/// tailwind --pwd ./project/pages --content "**/*.js"
/// tailwind --pwd ./project/components --content "**/*.js"
/// ```
pub fn optimize_patterns(entries: &Vec<GlobEntry>) -> Vec<GlobEntry> {
    let entries = hoist_static_glob_parts(entries, true);

    let mut pattern_map: FxHashMap<String, FxHashSet<String>> = FxHashMap::default();

    for glob_entry in entries {
        let entry = pattern_map.entry(glob_entry.base).or_default();
        entry.insert(glob_entry.pattern.clone());
    }

    let mut glob_entries = pattern_map
        .into_iter()
        .map(|(base, patterns)| {
            let size = patterns.len();

            let mut patterns = patterns.into_iter();

            GlobEntry {
                base,
                pattern: match size {
                    // SAFETY: we can unwrap here because we know that the size is 1.
                    1 => patterns.next().unwrap(),
                    _ => {
                        let mut patterns = patterns.collect::<Vec<_>>();

                        patterns.sort();

                        format!("{{{}}}", patterns.join(","))
                    }
                },
            }
        })
        .collect::<Vec<GlobEntry>>();

    glob_entries.sort_by(|a, z| a.base.cmp(&z.base));

    glob_entries
}

pub fn split_pattern(pattern: &str) -> (Option<String>, Option<String>) {
    if !pattern.contains('*') {
        return (Some(pattern.to_owned()), None);
    }

    let mut last_slash_position = None;

    for (i, c) in pattern.char_indices() {
        if c == '/' {
            last_slash_position = Some(i);
        }

        if c == '*' || c == '!' {
            break;
        }
    }

    let Some(last_slash_position) = last_slash_position else {
        return (None, Some(pattern.to_owned()));
    };

    let static_part = pattern[..last_slash_position].to_owned();
    let dynamic_part = pattern[last_slash_position + 1..].to_owned();

    let static_part = (!static_part.is_empty()).then_some(static_part);
    let dynamic_part = (!dynamic_part.is_empty()).then_some(dynamic_part);

    (static_part, dynamic_part)
}
