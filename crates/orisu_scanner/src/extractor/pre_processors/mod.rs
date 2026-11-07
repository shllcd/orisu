mod clojure;
mod elixir;
mod haml;
mod json;
mod markdown;
mod pug;
mod razor;
mod ruby;
mod rust;
mod slim;
mod svelte;
mod template_toolkit;
mod twig;
mod vue;

/// Rewrites a file of the language `extension` names so that the extractor finds the classes in it.
pub fn pre_process(content: Vec<u8>, extension: &str) -> Vec<u8> {
    match extension {
        "clj" | "cljs" | "cljc" => clojure::process(&content),
        "heex" | "eex" | "ex" | "exs" => elixir::process(&content),
        "cshtml" | "razor" => razor::process(&content),
        "haml" => haml::process(&content),
        "json" | "jsonl" | "ndjson" => json::process(&content),
        "md" | "mdx" => markdown::process(&content),
        "pug" => pug::process(&content),
        "rb" | "erb" => ruby::process(&content),
        "slim" | "slang" => slim::process(&content),
        "svelte" => svelte::process(&content),
        "rs" => rust::process(&content),
        "tt" | "tt2" | "tx" => template_toolkit::process(&content),
        "twig" => twig::process(&content),
        "vue" => vue::process(&content),
        _ => content,
    }
}
