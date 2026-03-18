pub(crate) const MAX_QUERY_CHARS: usize = 512;

pub(crate) fn normalize_for_search(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut previous_space = false;
    for character in text.chars() {
        if character.is_alphanumeric() {
            previous_space = false;
            for lower in character.to_lowercase() {
                normalized.push(lower);
            }
        } else if !previous_space {
            normalized.push(' ');
            previous_space = true;
        }
    }
    normalized.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_for_search;

    #[test]
    fn normalizes_case_and_punctuation() {
        let normalized = normalize_for_search("  Hello, WORLD!!  ");
        assert_eq!(normalized, "hello world");
    }

    #[test]
    fn collapses_symbol_runs_to_single_spaces() {
        let normalized = normalize_for_search("A&B---C///D");
        assert_eq!(normalized, "a b c d");
    }
}
