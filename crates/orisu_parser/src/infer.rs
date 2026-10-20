// Portions ported from Tailwind CSS (https://github.com/tailwindlabs/tailwindcss),
// MIT License, Copyright (c) Tailwind Labs, Inc.

use crate::math::has_math_fn;
use crate::segment::segment;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataType {
    Color,
    Length,
    Percentage,
    Ratio,
    Number,
    Integer,
    Url,
    Position,
    BgSize,
    LineWidth,
    Image,
    FamilyName,
    GenericName,
    AbsoluteSize,
    RelativeSize,
    Angle,
    Vector,
}

impl DataType {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "color" => Self::Color,
            "length" => Self::Length,
            "percentage" => Self::Percentage,
            "ratio" => Self::Ratio,
            "number" => Self::Number,
            "integer" => Self::Integer,
            "url" => Self::Url,
            "position" => Self::Position,
            "bg-size" => Self::BgSize,
            "line-width" => Self::LineWidth,
            "image" => Self::Image,
            "family-name" => Self::FamilyName,
            "generic-name" => Self::GenericName,
            "absolute-size" => Self::AbsoluteSize,
            "relative-size" => Self::RelativeSize,
            "angle" => Self::Angle,
            "vector" => Self::Vector,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Color => "color",
            Self::Length => "length",
            Self::Percentage => "percentage",
            Self::Ratio => "ratio",
            Self::Number => "number",
            Self::Integer => "integer",
            Self::Url => "url",
            Self::Position => "position",
            Self::BgSize => "bg-size",
            Self::LineWidth => "line-width",
            Self::Image => "image",
            Self::FamilyName => "family-name",
            Self::GenericName => "generic-name",
            Self::AbsoluteSize => "absolute-size",
            Self::RelativeSize => "relative-size",
            Self::Angle => "angle",
            Self::Vector => "vector",
        }
    }

    pub fn check(self, value: &str) -> bool {
        match self {
            Self::Color => is_color(value),
            Self::Length => is_length(value),
            Self::Percentage => is_percentage(value),
            Self::Ratio => is_fraction(value),
            Self::Number => is_number(value),
            Self::Integer => is_positive_integer(value),
            Self::Url => is_url(value),
            Self::Position => is_background_position(value),
            Self::BgSize => is_background_size(value),
            Self::LineWidth => is_line_width(value),
            Self::Image => is_image(value),
            Self::FamilyName => is_family_name(value),
            Self::GenericName => is_generic_name(value),
            Self::AbsoluteSize => is_absolute_size(value),
            Self::RelativeSize => matches!(value, "larger" | "smaller"),
            Self::Angle => is_angle(value),
            Self::Vector => is_vector(value),
        }
    }
}

pub fn infer_data_type(value: &str, types: &[DataType]) -> Option<DataType> {
    if value.starts_with("var(") {
        return None;
    }
    types.iter().copied().find(|t| t.check(value))
}

const NAMED_COLORS: &[&str] = &[
    "black",
    "silver",
    "gray",
    "white",
    "maroon",
    "red",
    "purple",
    "fuchsia",
    "green",
    "lime",
    "olive",
    "yellow",
    "navy",
    "blue",
    "teal",
    "aqua",
    "aliceblue",
    "antiquewhite",
    "aquamarine",
    "azure",
    "beige",
    "bisque",
    "blanchedalmond",
    "blueviolet",
    "brown",
    "burlywood",
    "cadetblue",
    "chartreuse",
    "chocolate",
    "coral",
    "cornflowerblue",
    "cornsilk",
    "crimson",
    "cyan",
    "darkblue",
    "darkcyan",
    "darkgoldenrod",
    "darkgray",
    "darkgreen",
    "darkgrey",
    "darkkhaki",
    "darkmagenta",
    "darkolivegreen",
    "darkorange",
    "darkorchid",
    "darkred",
    "darksalmon",
    "darkseagreen",
    "darkslateblue",
    "darkslategray",
    "darkslategrey",
    "darkturquoise",
    "darkviolet",
    "deeppink",
    "deepskyblue",
    "dimgray",
    "dimgrey",
    "dodgerblue",
    "firebrick",
    "floralwhite",
    "forestgreen",
    "gainsboro",
    "ghostwhite",
    "gold",
    "goldenrod",
    "greenyellow",
    "grey",
    "honeydew",
    "hotpink",
    "indianred",
    "indigo",
    "ivory",
    "khaki",
    "lavender",
    "lavenderblush",
    "lawngreen",
    "lemonchiffon",
    "lightblue",
    "lightcoral",
    "lightcyan",
    "lightgoldenrodyellow",
    "lightgray",
    "lightgreen",
    "lightgrey",
    "lightpink",
    "lightsalmon",
    "lightseagreen",
    "lightskyblue",
    "lightslategray",
    "lightslategrey",
    "lightsteelblue",
    "lightyellow",
    "limegreen",
    "linen",
    "magenta",
    "mediumaquamarine",
    "mediumblue",
    "mediumorchid",
    "mediumpurple",
    "mediumseagreen",
    "mediumslateblue",
    "mediumspringgreen",
    "mediumturquoise",
    "mediumvioletred",
    "midnightblue",
    "mintcream",
    "mistyrose",
    "moccasin",
    "navajowhite",
    "oldlace",
    "olivedrab",
    "orange",
    "orangered",
    "orchid",
    "palegoldenrod",
    "palegreen",
    "paleturquoise",
    "palevioletred",
    "papayawhip",
    "peachpuff",
    "peru",
    "pink",
    "plum",
    "powderblue",
    "rebeccapurple",
    "rosybrown",
    "royalblue",
    "saddlebrown",
    "salmon",
    "sandybrown",
    "seagreen",
    "seashell",
    "sienna",
    "skyblue",
    "slateblue",
    "slategray",
    "slategrey",
    "snow",
    "springgreen",
    "steelblue",
    "tan",
    "thistle",
    "tomato",
    "turquoise",
    "violet",
    "wheat",
    "whitesmoke",
    "yellowgreen",
    "transparent",
    "currentcolor",
    "canvas",
    "canvastext",
    "linktext",
    "visitedtext",
    "activetext",
    "buttonface",
    "buttontext",
    "buttonborder",
    "field",
    "fieldtext",
    "highlight",
    "highlighttext",
    "selecteditem",
    "selecteditemtext",
    "mark",
    "marktext",
    "graytext",
    "accentcolor",
    "accentcolortext",
];

pub fn is_named_color(value: &str) -> bool {
    value.len() <= 20 && NAMED_COLORS.iter().any(|c| c.eq_ignore_ascii_case(value))
}

const COLOR_FNS: &[&str] =
    &["rgb", "rgba", "hsl", "hsla", "hwb", "color", "lab", "lch", "oklab", "oklch", "light-dark", "color-mix", "--alpha"];

pub fn is_color(value: &str) -> bool {
    if value.starts_with('#') {
        return true;
    }
    if let Some(paren) = value.find('(') {
        let name = &value[..paren];
        if COLOR_FNS.iter().any(|f| f.eq_ignore_ascii_case(name)) {
            return true;
        }
    }
    is_named_color(value)
}

fn is_url(value: &str) -> bool {
    value.starts_with("url(") && value.ends_with(')') && !value[4..].contains('\n')
}

fn is_line_width(value: &str) -> bool {
    segment(value, b' ').iter().all(|v| is_length(v) || is_number(v) || matches!(*v, "thin" | "medium" | "thick"))
}

fn is_gradient_fn(part: &str) -> bool {
    let rest = part.strip_prefix("repeating-").unwrap_or(part);
    ["conic-gradient(", "linear-gradient(", "radial-gradient("].iter().any(|p| rest.starts_with(p))
}

fn is_image(value: &str) -> bool {
    let mut count = 0;
    for part in segment(value, b',') {
        if part.starts_with("var(") {
            continue;
        }
        if is_url(part) || is_gradient_fn(part) || ["element(", "image(", "cross-fade(", "image-set("].iter().any(|p| part.starts_with(p)) {
            count += 1;
            continue;
        }
        return false;
    }
    count > 0
}

fn is_generic_name(value: &str) -> bool {
    matches!(
        value,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "math"
            | "emoji"
            | "fangsong"
    )
}

fn is_family_name(value: &str) -> bool {
    let mut count = 0;
    for part in segment(value, b',') {
        if part.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            return false;
        }
        if part.starts_with("var(") {
            continue;
        }
        count += 1;
    }
    count > 0
}

fn is_absolute_size(value: &str) -> bool {
    matches!(value, "xx-small" | "x-small" | "small" | "medium" | "large" | "x-large" | "xx-large" | "xxx-large")
}

pub fn match_number(s: &[u8]) -> Option<usize> {
    let mut i = 0;
    if matches!(s.first(), Some(b'+' | b'-')) {
        i += 1;
    }
    let int_start = i;
    while i < s.len() && s[i].is_ascii_digit() {
        i += 1;
    }
    let int_len = i - int_start;
    if s.get(i) == Some(&b'.') && s.get(i + 1).is_some_and(u8::is_ascii_digit) {
        i += 1;
        while i < s.len() && s[i].is_ascii_digit() {
            i += 1;
        }
    } else if int_len == 0 {
        return None;
    }
    if matches!(s.get(i), Some(b'e' | b'E')) {
        let mut j = i + 1;
        if matches!(s.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
        if s.get(j).is_some_and(u8::is_ascii_digit) {
            while j < s.len() && s[j].is_ascii_digit() {
                j += 1;
            }
            i = j;
        }
    }
    Some(i)
}

fn is_number_then(value: &str, units: &[&str]) -> bool {
    let bytes = value.as_bytes();
    match match_number(bytes) {
        Some(end) => units.contains(&&value[end..]),
        None => false,
    }
}

pub fn is_plain_number(value: &str) -> bool {
    match_number(value.as_bytes()) == Some(value.len())
}

fn is_number(value: &str) -> bool {
    is_plain_number(value) || has_math_fn(value)
}

fn is_percentage(value: &str) -> bool {
    is_number_then(value, &["%"]) || has_math_fn(value)
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while i < s.len() && matches!(s[i], b' ' | b'\t' | b'\n' | b'\r' | 0x0c) {
        i += 1;
    }
    i
}

fn is_fraction(value: &str) -> bool {
    let s = value.as_bytes();
    let check = || {
        let a = match_number(s)?;
        let i = skip_ws(s, a);
        if s.get(i) != Some(&b'/') {
            return None;
        }
        let i = skip_ws(s, i + 1);
        let b = match_number(&s[i..])?;
        (i + b == s.len()).then_some(())
    };
    check().is_some() || has_math_fn(value)
}

const LENGTH_UNITS: &[&str] = &[
    "cm", "mm", "Q", "in", "pc", "pt", "px", "em", "ex", "ch", "rem", "lh", "rlh", "vw", "vh", "vmin", "vmax", "vb", "vi", "svw", "svh",
    "lvw", "lvh", "dvw", "dvh", "cqw", "cqh", "cqi", "cqb", "cqmin", "cqmax",
];

pub fn is_length(value: &str) -> bool {
    is_number_then(value, LENGTH_UNITS) || value.get(..10).is_some_and(|p| p.eq_ignore_ascii_case("--spacing(")) || has_math_fn(value)
}

fn is_background_position(value: &str) -> bool {
    let mut count = 0;
    for part in segment(value, b' ') {
        if matches!(part, "center" | "top" | "right" | "bottom" | "left") {
            count += 1;
            continue;
        }
        if part.starts_with("var(") {
            continue;
        }
        if is_length(part) || is_percentage(part) {
            count += 1;
            continue;
        }
        return false;
    }
    count > 0
}

fn is_background_size(value: &str) -> bool {
    let mut count = 0;
    for size in segment(value, b',') {
        if size == "cover" || size == "contain" {
            count += 1;
            continue;
        }
        let values = segment(size, b' ');
        if values.len() != 1 && values.len() != 2 {
            return false;
        }
        if values.iter().all(|v| *v == "auto" || is_length(v) || is_percentage(v)) {
            count += 1;
        }
    }
    count > 0
}

fn is_angle(value: &str) -> bool {
    is_number_then(value, &["deg", "rad", "grad", "turn"])
}

fn is_vector(value: &str) -> bool {
    let s = value.as_bytes();
    let mut i = 0;
    for n in 0..3 {
        if n > 0 {
            let start = i;
            while s.get(i) == Some(&b' ') {
                i += 1;
            }
            if i == start {
                return false;
            }
        }
        match match_number(&s[i..]) {
            Some(len) => i += len,
            None => return false,
        }
    }
    i == s.len()
}

/// JavaScript `Number(value)`: `NaN` when the string is not numeric.
pub fn js_number(value: &str) -> f64 {
    let s = orisu_ast::bytes::trim(value);
    if s.is_empty() {
        return 0.0;
    }
    let b = s.as_bytes();
    if b.len() > 2 && b[0] == b'0' {
        let radix = match b[1] {
            b'x' | b'X' => 16,
            b'o' | b'O' => 8,
            b'b' | b'B' => 2,
            _ => 0,
        };
        if radix != 0 {
            return u128::from_str_radix(&s[2..], radix).map_or(f64::NAN, |n| n as f64);
        }
    }
    let unsigned = s.strip_prefix(['+', '-']).unwrap_or(s);
    if unsigned == "Infinity" {
        return if b[0] == b'-' { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if !unsigned.bytes().all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-')) {
        return f64::NAN;
    }
    s.parse().unwrap_or(f64::NAN)
}

pub fn parse_js_number(value: &str) -> Option<f64> {
    let n = js_number(value);
    n.is_finite().then_some(n)
}

pub fn format_js_number(n: f64) -> String {
    if n == 0.0 {
        return "0".to_owned();
    }
    if !n.is_finite() {
        return if n.is_nan() {
            "NaN".into()
        } else if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    let abs = n.abs();
    if (1e-7..1e21).contains(&abs) {
        return format!("{n}");
    }
    let s = format!("{n:e}");
    match s.split_once('e') {
        Some((m, e)) if !e.starts_with('-') => format!("{m}e+{e}"),
        Some((m, e)) => format!("{m}e{e}"),
        None => s,
    }
}

fn is_canonical(value: &str) -> Option<f64> {
    if is_short_decimal(value) {
        return parse_js_number(value);
    }
    let n = parse_js_number(value)?;
    (format_js_number(n) == value).then_some(n)
}

/// Whether `value` is a decimal that `format_js_number` reproduces unchanged: at most 15 significant
/// digits, no redundant zeros and not small enough for exponent notation.
fn is_short_decimal(value: &str) -> bool {
    let (int, frac) = value.split_once('.').unwrap_or((value, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || !digits(int) || !digits(frac) || (int.len() > 1 && int.starts_with('0')) {
        return false;
    }
    if value.len() > int.len() && (frac.is_empty() || frac.ends_with('0')) {
        return false;
    }
    if int == "0" {
        let zeros = frac.bytes().take_while(|&b| b == b'0').count();
        frac.is_empty() || (zeros <= 6 && frac.len() - zeros <= 15)
    } else {
        int.len() + frac.len() <= 15
    }
}

pub fn is_positive_integer(value: &str) -> bool {
    is_canonical(value).is_some_and(|n| n.fract() == 0.0 && n >= 0.0)
}

pub fn is_strict_positive_integer(value: &str) -> bool {
    is_canonical(value).is_some_and(|n| n.fract() == 0.0 && n > 0.0)
}

pub fn is_multiple_of_quarter(value: &str) -> bool {
    is_canonical(value).is_some_and(|n| n >= 0.0 && n % 0.25 == 0.0)
}
