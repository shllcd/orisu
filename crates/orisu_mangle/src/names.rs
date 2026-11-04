const ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-";
const FIRST: u64 = 52;
const REST: u64 = 64;

/// The `k`-th short name: every name of one character, then of two, and so on. The first character is a letter, so
/// a name is a CSS identifier that needs no escape.
pub fn name(mut k: u64) -> String {
    let mut len = 1;
    let mut count = FIRST;
    while k >= count {
        k -= count;
        len += 1;
        count *= REST;
    }
    let mut out = vec![0; len];
    for slot in out[1..].iter_mut().rev() {
        *slot = ALPHABET[(k % REST) as usize];
        k /= REST;
    }
    out[0] = ALPHABET[k as usize];
    String::from_utf8(out).expect("the alphabet is ASCII")
}

/// Whether `prefix` keeps every `prefix + name` a CSS identifier without escapes.
pub fn valid_prefix(prefix: &str) -> bool {
    let bytes = prefix.as_bytes();
    bytes.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        && !bytes.first().is_some_and(u8::is_ascii_digit)
        && !(bytes.first() == Some(&b'-') && bytes.get(1).is_some_and(u8::is_ascii_digit))
}
