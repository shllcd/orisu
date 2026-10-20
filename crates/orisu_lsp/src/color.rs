//! Turns CSS color values into sRGB for swatches.

use orisu_parser::segment::segment;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub fn hex(&self) -> String {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        let mut out = format!("#{:02x}{:02x}{:02x}", byte(self.r), byte(self.g), byte(self.b));
        if self.a < 1.0 {
            out.push_str(&format!("{:02x}", byte(self.a)));
        }
        out
    }
}

/// Parses `value`, looking `var(--name)` up through `variable`.
pub fn parse<'v>(value: &str, variable: &dyn Fn(&str) -> Option<&'v str>) -> Option<Rgba> {
    parse_depth(value.trim(), variable, 0)
}

fn parse_depth<'v>(value: &str, variable: &dyn Fn(&str) -> Option<&'v str>, depth: u8) -> Option<Rgba> {
    if depth > 8 {
        return None;
    }
    let (name, args) = match value.split_once('(') {
        Some((name, rest)) => (name.trim().to_ascii_lowercase(), rest.strip_suffix(')')?),
        None => return keyword(&value.to_ascii_lowercase()),
    };
    match name.as_str() {
        "var" => {
            let parts = segment(args, b',');
            let name = parts.first()?.trim();
            match variable(name) {
                Some(resolved) => parse_depth(resolved.trim(), variable, depth + 1),
                None => parse_depth(parts.get(1)?.trim(), variable, depth + 1),
            }
        }
        "color-mix" => {
            let parts = segment(args, b',');
            let (color, amount) = mix_part(parts.get(1)?);
            let other = parts.get(2).map(|p| mix_part(p).0);
            let first = parse_depth(color, variable, depth + 1)?;
            match other.map(|o| parse_depth(o, variable, depth + 1)) {
                Some(Some(second)) if second.a > 0.0 => {
                    let t = amount.unwrap_or(0.5);
                    let lerp = |a: f32, b: f32| a * t + b * (1.0 - t);
                    Some(Rgba {
                        r: lerp(first.r, second.r),
                        g: lerp(first.g, second.g),
                        b: lerp(first.b, second.b),
                        a: lerp(first.a, second.a),
                    })
                }
                _ => Some(Rgba { a: first.a * amount.unwrap_or(0.5), ..first }),
            }
        }
        "rgb" | "rgba" => {
            let (channels, alpha) = channels(args)?;
            let [r, g, b] = channels.map(|c| number(c, 255.0));
            Some(Rgba { r: r? / 255.0, g: g? / 255.0, b: b? / 255.0, a: alpha })
        }
        "hsl" | "hsla" => {
            let (channels, alpha) = channels(args)?;
            let h = hue(channels[0])?;
            let s = number(channels[1], 100.0)? / 100.0;
            let l = number(channels[2], 100.0)? / 100.0;
            let f = |n: f32| {
                let k = (n + h / 30.0) % 12.0;
                l - s * l.min(1.0 - l) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
            };
            Some(Rgba { r: f(0.0), g: f(8.0), b: f(4.0), a: alpha })
        }
        "oklch" => {
            let (channels, alpha) = channels(args)?;
            let l = number(channels[0], 1.0)?;
            let c = number(channels[1], 0.4)?;
            let h = hue(channels[2])?.to_radians();
            Some(oklab(l, c * h.cos(), c * h.sin(), alpha))
        }
        "oklab" => {
            let (channels, alpha) = channels(args)?;
            Some(oklab(number(channels[0], 1.0)?, number(channels[1], 0.4)?, number(channels[2], 0.4)?, alpha))
        }
        _ => None,
    }
}

fn keyword(value: &str) -> Option<Rgba> {
    if let Some(hex) = value.strip_prefix('#') {
        let digits: Vec<f32> = hex.chars().map(|c| c.to_digit(16).map(|d| d as f32)).collect::<Option<_>>()?;
        let (rgb, a) = match digits.len() {
            3 | 4 => ([digits[0], digits[1], digits[2]].map(|d| d * 17.0 / 255.0), digits.get(3).map_or(1.0, |d| d * 17.0 / 255.0)),
            6 | 8 => {
                let pair = |i: usize| (digits[i] * 16.0 + digits[i + 1]) / 255.0;
                ([pair(0), pair(2), pair(4)], if digits.len() == 8 { pair(6) } else { 1.0 })
            }
            _ => return None,
        };
        return Some(Rgba { r: rgb[0], g: rgb[1], b: rgb[2], a });
    }
    let [r, g, b, a] = match value {
        "black" => [0.0, 0.0, 0.0, 1.0],
        "white" => [1.0, 1.0, 1.0, 1.0],
        "transparent" => [0.0, 0.0, 0.0, 0.0],
        _ => return None,
    };
    Some(Rgba { r, g, b, a })
}

/// The color and percentage of a `color-mix()` argument such as `var(--x) 50%`.
fn mix_part(part: &str) -> (&str, Option<f32>) {
    let part = part.trim();
    match part.rsplit_once(char::is_whitespace) {
        Some((color, amount)) if amount.ends_with('%') => (color.trim(), number(amount, 1.0)),
        _ => (part, None),
    }
}

/// Splits `a b c / alpha` or `a, b, c, alpha`.
fn channels(args: &str) -> Option<([&str; 3], f32)> {
    let (main, alpha) = match args.split_once('/') {
        Some((main, alpha)) => (main, Some(alpha.trim())),
        None => (args, None),
    };
    let mut parts: Vec<&str> = main.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    let alpha = match alpha {
        Some(alpha) => Some(alpha),
        None if parts.len() == 4 => parts.pop(),
        None => None,
    };
    let alpha = alpha.map_or(Some(1.0), |a| number(a, 1.0))?;
    (parts.len() == 3).then(|| ([parts[0], parts[1], parts[2]], alpha))
}

/// A number, with `%` taken relative to `full`; `none` is zero.
fn number(value: &str, full: f32) -> Option<f32> {
    if value == "none" {
        return Some(0.0);
    }
    match value.strip_suffix('%') {
        Some(percent) => percent.parse::<f32>().ok().map(|p| p / 100.0 * full),
        None => value.parse().ok(),
    }
}

fn hue(value: &str) -> Option<f32> {
    number(value.trim_end_matches("deg"), 360.0)
}

fn oklab(l: f32, a: f32, b: f32, alpha: f32) -> Rgba {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    let gamma = |x: f32| {
        let x = x.clamp(0.0, 1.0);
        if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
    };
    Rgba {
        r: gamma(4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_),
        g: gamma(-1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_),
        b: gamma(-0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_),
        a: alpha,
    }
}
