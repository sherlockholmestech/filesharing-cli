pub fn normalize(token: Option<String>) -> Option<String> {
    token.and_then(|value| normalize_ref(Some(&value)).map(str::to_owned))
}

pub fn normalize_ref(token: Option<&str>) -> Option<&str> {
    token.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::{normalize, normalize_ref};

    #[test]
    fn normalizes_owned_token() {
        assert_eq!(
            normalize(Some("  abc123  ".to_string())),
            Some("abc123".to_string())
        );
        assert_eq!(normalize(Some("\t\n ".to_string())), None);
        assert_eq!(normalize(None), None);
    }

    #[test]
    fn normalizes_borrowed_token() {
        assert_eq!(normalize_ref(Some("  key  ")), Some("key"));
        assert_eq!(normalize_ref(Some("")), None);
        assert_eq!(normalize_ref(None), None);
    }
}
