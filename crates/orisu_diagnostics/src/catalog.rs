use crate::Severity;
use Severity::{Error, Warn};

/// Source in `lang` where `bad` reports the code and `good` does not.
pub struct Example {
    pub lang: &'static str,
    pub bad: &'static str,
    pub good: &'static str,
}

pub struct Entry {
    pub code: Code,
    pub name: &'static str,
    pub severity: Severity,
    pub title: &'static str,
    /// `{key}` placeholders are filled from the report's arguments.
    pub message: &'static str,
    pub explanation: &'static str,
    pub repair: &'static str,
    /// Title template of the automatic fix, when the code can have one.
    pub fix: Option<&'static str>,
    pub example: Example,
}

macro_rules! catalog {
    ($(
        $variant:ident {
            name: $name:literal,
            severity: $severity:ident,
            title: $title:literal,
            message: $message:literal,
            explanation: $explanation:literal,
            repair: $repair:literal,
            fix: $fix:expr,
            example: $example:expr,
        }
    )*) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum Code {
            $($variant),*
        }

        /// Every code in declaration order: errors, then warnings.
        pub static CATALOG: &[Entry] = &[$(
            Entry {
                code: Code::$variant,
                name: $name,
                severity: $severity,
                title: $title,
                message: $message,
                explanation: $explanation,
                repair: $repair,
                fix: $fix,
                example: $example,
            }
        ),*];

        impl Code {
            pub fn entry(self) -> &'static Entry {
                &CATALOG[self as usize]
            }

            pub fn name(self) -> &'static str {
                match self {
                    $(Code::$variant => $name),*
                }
            }

            pub fn severity(self) -> Severity {
                match self {
                    $(Code::$variant => $severity),*
                }
            }
        }
    };
}

impl serde::Serialize for Code {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

catalog! {
    UnknownUtility {
        name: "UNKNOWN_UTILITY",
        severity: Error,
        title: "Class that generates no CSS",
        message: "`{class}` is not a utility, so it generates no CSS.",
        explanation: "No utility, `@utility` or arbitrary property matches the class, so the stylesheet has no rule for it. `data.class` is the class. When a known utility is a close spelling, the report carries a fix and `data.suggestion` is that utility.",
        repair: "Apply the fix when there is one. Otherwise check the spelling against the theme, declare the utility with `@utility`, or write an arbitrary value such as `w-[13px]`.",
        fix: Some("replace with `{suggestion}`"),
        example: Example {
            lang: "html",
            bad: "<div class=\"flex itms-center\"></div>\n",
            good: "<div class=\"flex items-center\"></div>\n",
        },
    }
    UnknownVariant {
        name: "UNKNOWN_VARIANT",
        severity: Error,
        title: "Unknown variant",
        message: "`{variant}:` is not a variant, so `{class}` generates no CSS.",
        explanation: "Every variant of a class must be known: built in, declared with `@custom-variant`, or arbitrary such as `[&>*]:`. An unknown one drops the whole class. `data.variant` is the variant and `data.class` the class.",
        repair: "Fix the spelling, declare the variant with `@custom-variant`, or write it as an arbitrary variant.",
        fix: None,
        example: Example {
            lang: "html",
            bad: "<a class=\"hovr:underline\"></a>\n",
            good: "<a class=\"hover:underline\"></a>\n",
        },
    }
    UnsupportedPlugin {
        name: "UNSUPPORTED_PLUGIN",
        severity: Error,
        title: "JavaScript plugin or config",
        message: "`@{directive} \"{id}\"` loads JavaScript, which orisu does not run.{hint}",
        explanation: "orisu compiles CSS without a JavaScript runtime, so `@plugin` and `@config` are rejected and the stylesheet does not build. `data.directive` is `plugin` or `config`, `data.id` the module, and `data.hint` names the CSS preset that replaces a known plugin, or is empty.",
        repair: "Import the preset named in the message instead of the plugin. Otherwise move the configuration into `@theme`, `@utility` and `@custom-variant`.",
        fix: None,
        example: Example {
            lang: "css",
            bad: "@import \"tailwindcss\";\n@plugin \"@tailwindcss/typography\";\n",
            good: "@import \"tailwindcss\";\n@import \"orisu/presets/tailwind-typography.css\";\n",
        },
    }
    ConflictingClasses {
        name: "CONFLICTING_CLASSES",
        severity: Warn,
        title: "Classes that set the same properties",
        message: "`{class}` sets the same CSS properties as {others}, so only one of them takes effect.",
        explanation: "Both classes apply under the same variants and write the same properties. The one later in the stylesheet wins regardless of order in the attribute. `data.class` is this class and `data.others` the conflicting ones, each in backticks.",
        repair: "Keep one of the classes. To let a later class override an earlier one at runtime, combine them with `cn` from `orisu/cn`.",
        fix: None,
        example: Example {
            lang: "html",
            bad: "<div class=\"p-2 p-4\"></div>\n",
            good: "<div class=\"p-4\"></div>\n",
        },
    }
    InvalidThemeKey {
        name: "INVALID_THEME_KEY",
        severity: Warn,
        title: "Reference to a missing theme key",
        message: "`{key}` is defined neither in the theme nor in a stylesheet, so `{class}` reads an empty value.",
        explanation: "The class reads a custom property through `(--key)` or `var(--key)`. The key sits in a theme namespace such as `--color-*` or `--spacing-*`, yet `@theme` and the project's stylesheets never define it, which usually means a misspelling. `data.key` is the variable and `data.class` the class.",
        repair: "Fix the spelling, add the key under `@theme`, or define the property where the element can inherit it.",
        fix: None,
        example: Example {
            lang: "html",
            bad: "<p class=\"text-(--color-brnd)\"></p>\n",
            good: "<p class=\"text-(--color-brand)\"></p>\n",
        },
    }
    NonCanonicalClass {
        name: "NON_CANONICAL_CLASS",
        severity: Warn,
        title: "Class with a shorter spelling",
        message: "`{class}` can be written as `{canonical}`.",
        explanation: "The class generates the same CSS as a shorter or theme-based spelling. `data.class` is the class and `data.canonical` the replacement.",
        repair: "Apply the fix.",
        fix: Some("replace with `{canonical}`"),
        example: Example {
            lang: "html",
            bad: "<div class=\"w-[16px]\"></div>\n",
            good: "<div class=\"w-4\"></div>\n",
        },
    }
}
