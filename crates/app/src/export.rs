//! CSV / JSON serialization for query and table results.
//! Kept here (no egui) so it can be unit-tested via the lib target.

/// Render headers + rows as CSV (RFC-ish: quote fields containing a comma,
/// quote or newline; `NULL` cells become empty).
pub fn to_csv(headers: &[String], rows: &[Vec<Option<String>>]) -> String {
    let mut out = String::new();
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    push_csv_row(&mut out, &header_refs);
    for row in rows {
        let fields: Vec<&str> = row.iter().map(|c| c.as_deref().unwrap_or("")).collect();
        push_csv_row(&mut out, &fields);
    }
    out
}

fn push_csv_row(out: &mut String, fields: &[&str]) {
    for (i, field) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let needs_quotes = field.contains(',') || field.contains('"') || field.contains('\n');
        if needs_quotes {
            out.push('"');
            for ch in field.chars() {
                if ch == '"' {
                    out.push('"');
                }
                out.push(ch);
            }
            out.push('"');
        } else {
            out.push_str(field);
        }
    }
    out.push('\n');
}

/// Render headers + rows as a JSON array of objects.
pub fn to_json(headers: &[String], rows: &[Vec<Option<String>>]) -> String {
    let mut out = String::from("[");
    for (r, row) in rows.iter().enumerate() {
        if r > 0 {
            out.push(',');
        }
        out.push('{');
        for (c, name) in headers.iter().enumerate() {
            if c > 0 {
                out.push(',');
            }
            out.push('"');
            push_json_escaped(&mut out, name);
            out.push('"');
            out.push(':');
            match row.get(c) {
                Some(Some(value)) => {
                    out.push('"');
                    push_json_escaped(&mut out, value);
                    out.push('"');
                }
                _ => out.push_str("null"),
            }
        }
        out.push('}');
    }
    out.push(']');
    out
}

fn push_json_escaped(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> Vec<String> {
        ["id", "name", "note"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn rows() -> Vec<Vec<Option<String>>> {
        vec![
            vec![Some("1".into()), Some("Ada, \"A.\"".into()), None],
            vec![
                Some("2".into()),
                Some("Grace".into()),
                Some("note\nline2".into()),
            ],
        ]
    }

    #[test]
    fn csv_escapes_quotes_commas_and_newlines() {
        let csv = to_csv(&headers(), &rows());
        assert!(csv.starts_with("id,name,note\n"), "got {csv:?}");
        assert!(csv.contains("1,\"Ada, \"\"A.\"\"\",\n"), "got {csv:?}");
        assert!(csv.contains("2,Grace,\"note\nline2\"\n"), "got {csv:?}");
    }

    #[test]
    fn json_uses_null_for_null_cells() {
        let json = to_json(&headers(), &rows());
        assert!(json.contains("\"id\":\"1\""), "got {json}");
        assert!(json.contains("\"note\":null"), "got {json}");
        assert!(json.contains("\"name\":\"Ada, \\\"A.\\\"\""));
    }
}
