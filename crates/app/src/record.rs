//! Record-form helpers (default handling), mirroring tup-db-client's Add form
//! pre-fill behaviour but without leaking Postgres cast expressions.

/// Turn a stored `column_default` into something safe to pre-fill a form with.
///
/// Postgres stores defaults as expressions — `'pending'::character varying`,
/// `now()`, `nextval('x'::regclass)`, `0` … We only pre-fill *plain literals*
/// (quoted strings are unquoted, numbers/booleans kept); anything computed by
/// the server (`now()`, sequences, functions) becomes `None` so saving leaves
/// the field out and the database applies the default itself.
pub fn clean_default(default: &str) -> Option<String> {
    let d = default.trim();
    if let Some(literal) = unquote_string_literal(d) {
        return Some(literal);
    }
    let is_numeric = !d.is_empty() && d.chars().all(|c| c.is_ascii_digit() || "+-.".contains(c));
    if is_numeric {
        return Some(d.to_owned());
    }
    match d.to_ascii_lowercase().as_str() {
        "true" | "false" | "null" => Some(d.to_owned()),
        _ => None,
    }
}

/// Extract `'value'` from `'value'::type` (or any trailing cast), handling
/// doubled quotes. Returns `None` when not a quoted string literal.
fn unquote_string_literal(default: &str) -> Option<String> {
    let s = default.trim();
    if !s.starts_with('\'') {
        return None;
    }
    let mut out = String::new();
    let mut chars = s[1..].chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                out.push('\'');
                chars.next();
                continue;
            }
            return Some(out); // closing quote; ignore `::type`
        }
        out.push(c);
    }
    None // unterminated literal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unquotes_cast_literals() {
        assert_eq!(
            clean_default("'pending'::character varying"),
            Some("pending".into())
        );
        assert_eq!(clean_default("'active'::text"), Some("active".into()));
        assert_eq!(clean_default("'a''b'::text"), Some("a'b".into()));
        assert_eq!(
            clean_default("'   padded '::character varying"),
            Some("   padded ".into())
        );
    }

    #[test]
    fn keeps_bare_literals() {
        assert_eq!(clean_default("0"), Some("0".into()));
        assert_eq!(clean_default("-1.5"), Some("-1.5".into()));
        assert_eq!(clean_default("true"), Some("true".into()));
        assert_eq!(clean_default("FALSE"), Some("FALSE".into()));
    }

    #[test]
    fn skips_function_defaults() {
        assert_eq!(clean_default("now()"), None);
        assert_eq!(clean_default("CURRENT_TIMESTAMP"), None);
        assert_eq!(clean_default("nextval('customers_id_seq'::regclass)"), None);
        assert_eq!(clean_default("gen_random_uuid()"), None);
    }
}
