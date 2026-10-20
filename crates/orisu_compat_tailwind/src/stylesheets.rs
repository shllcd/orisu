pub const INDEX: &str = include_str!("../assets/index.css");
pub const PREFLIGHT: &str = include_str!("../assets/preflight.css");
pub const THEME: &str = include_str!("../assets/theme.css");
pub const UTILITIES: &str = include_str!("../assets/utilities.css");

pub fn builtin(id: &str) -> Option<&'static str> {
    Some(match id {
        "tailwindcss" | "tailwindcss/index.css" => INDEX,
        "tailwindcss/preflight" | "tailwindcss/preflight.css" => PREFLIGHT,
        "tailwindcss/theme" | "tailwindcss/theme.css" => THEME,
        "tailwindcss/utilities" | "tailwindcss/utilities.css" => UTILITIES,
        _ => return None,
    })
}
