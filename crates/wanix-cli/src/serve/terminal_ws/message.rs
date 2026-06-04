pub(in crate::serve) fn parse_terminal_resize_message(text: &str) -> Option<(u16, u16)> {
    parse_terminal_resize_json(text).or_else(|| parse_terminal_resize_text(text))
}

fn parse_terminal_resize_json(text: &str) -> Option<(u16, u16)> {
    let compact = compact_json(text);
    require_json_resize_type(&compact)?;
    let columns = json_u16_field(&compact, "columns")?;
    let rows = json_u16_field(&compact, "rows")?;
    Some((columns, rows))
}

fn parse_terminal_resize_text(text: &str) -> Option<(u16, u16)> {
    let parts = resize_text_parts(text)?;
    let columns = parse_resize_dimension(parts.columns)?;
    let rows = parse_resize_dimension(parts.rows)?;
    Some((columns, rows))
}

fn compact_json(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn require_json_resize_type(compact_json: &str) -> Option<()> {
    compact_json.contains("\"type\":\"resize\"").then_some(())
}

fn json_u16_field(compact_json: &str, field: &str) -> Option<u16> {
    let marker = format!("\"{field}\":");
    let start = compact_json.find(&marker)? + marker.len();
    nonzero_u16(read_ascii_u64(&compact_json[start..])?)
}

fn read_ascii_u64(text: &str) -> Option<u64> {
    let digits = text
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    (!digits.is_empty()).then(|| digits.parse().ok())?
}

fn parse_resize_dimension(value: &str) -> Option<u16> {
    nonzero_u16(value.parse().ok()?)
}

fn nonzero_u16(value: u64) -> Option<u16> {
    let value = u16::try_from(value).ok()?;
    (value != 0).then_some(value)
}

fn require_resize_command(command: &str) -> Option<()> {
    (command == "resize").then_some(())
}

fn require_no_extra_resize_parts(extra: Option<&str>) -> Option<()> {
    extra.is_none().then_some(())
}

struct ResizeTextParts<'a> {
    columns: &'a str,
    rows: &'a str,
}

fn resize_text_parts(text: &str) -> Option<ResizeTextParts<'_>> {
    let mut parts = text.split_whitespace();
    require_resize_command(parts.next()?)?;
    let columns = parts.next()?;
    let rows = parts.next()?;
    require_no_extra_resize_parts(parts.next())?;
    Some(ResizeTextParts { columns, rows })
}
