pub(in crate::serve) fn parse_terminal_resize_message(text: &str) -> Option<(u16, u16)> {
    if let Some(resize) = parse_terminal_resize_json(text) {
        return Some(resize);
    }
    let mut parts = text.split_whitespace();
    if parts.next()? != "resize" {
        return None;
    }
    let columns = parts.next()?.parse().ok()?;
    let rows = parts.next()?.parse().ok()?;
    if parts.next().is_some() || columns == 0 || rows == 0 {
        return None;
    }
    Some((columns, rows))
}

fn parse_terminal_resize_json(text: &str) -> Option<(u16, u16)> {
    let compact = text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if !compact.contains("\"type\":\"resize\"") {
        return None;
    }
    let columns = json_u16_field(&compact, "columns")?;
    let rows = json_u16_field(&compact, "rows")?;
    Some((columns, rows))
}

fn json_u16_field(compact_json: &str, field: &str) -> Option<u16> {
    let marker = format!("\"{field}\":");
    let start = compact_json.find(&marker)? + marker.len();
    let digits = compact_json[start..]
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    let value = digits.parse().ok()?;
    (value != 0).then_some(value)
}
