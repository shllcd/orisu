const INDEX: &str = include_str!("../assets/index.css");
const PREFLIGHT: &str = include_str!("../assets/preflight.css");
const THEME: &str = include_str!("../assets/theme.css");
const UTILITIES: &str = include_str!("../assets/utilities.css");

pub fn builtin(id: &str) -> Option<&'static str> {
    Some(match id {
        "tailwindcss" | "tailwindcss/index.css" => INDEX,
        "tailwindcss/preflight" | "tailwindcss/preflight.css" => PREFLIGHT,
        "tailwindcss/theme" | "tailwindcss/theme.css" => THEME,
        "tailwindcss/utilities" | "tailwindcss/utilities.css" => UTILITIES,
        _ => return None,
    })
}
