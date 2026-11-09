//! The one rule for a colour a user can choose.

/// `#rrggbb`, stored lower-case. Anything else is not a colour.
pub fn normalize_hex_color(value: &str) -> Option<String> {
    let text = value.trim();
    let ok = text.len() == 7
        && text.starts_with('#')
        && text[1..].bytes().all(|b| b.is_ascii_hexdigit());
    ok.then(|| text.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_colors() {
        assert_eq!(normalize_hex_color(" #A1b2C3 ").as_deref(), Some("#a1b2c3"));
        assert_eq!(normalize_hex_color("#abc"), None);
        assert_eq!(normalize_hex_color("red"), None);
    }
}
