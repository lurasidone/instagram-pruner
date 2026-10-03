/// Drops control characters and zero-width spaces, then caps the text at `max`
/// characters, appending an ellipsis when it had to cut.
pub fn truncate(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(max + 4);
    let mut taken = 0;
    for c in s.chars() {
        if c.is_control() || c == '\u{200b}' {
            continue;
        }
        if taken == max {
            out.push('\u{2026}');
            break;
        }
        out.push(c);
        taken += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn keeps_max_characters_without_ellipsis() {
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "abc\u{2026}");
        assert_eq!(truncate("", 3), "");
    }

    #[test]
    fn skipped_characters_do_not_count_towards_the_limit() {
        assert_eq!(truncate("a\nb\u{200b}c", 3), "abc");
        assert_eq!(truncate("a\nbc d", 3), "abc\u{2026}");
    }
}
