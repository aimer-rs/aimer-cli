use super::*;

pub(super) fn lsp_position(source: &str, line: u32, column: u32) -> anyhow::Result<Value> {
    if line == 0 || column == 0 {
        anyhow::bail!("line and column are 1-based and must be greater than zero");
    }
    let line_text = source
        .lines()
        .nth((line - 1) as usize)
        .ok_or_else(|| anyhow::anyhow!("line is outside the source file"))?;
    let requested_units = (column - 1) as usize;
    let line_length = line_text.encode_utf16().count();
    if requested_units > line_length {
        anyhow::bail!(
            "column {column} is outside source line {line}; line_length is {line_length} UTF-16 code units (valid columns are 1 through {})",
            line_length + 1
        );
    }
    let mut units = 0;
    for ch in line_text.chars() {
        if units == requested_units {
            break;
        }
        units += ch.len_utf16();
        if units > requested_units {
            anyhow::bail!(
                "column {column} splits a UTF-16 surrogate pair on source line {line}; line_length is {line_length} code units"
            );
        }
    }
    debug_assert_eq!(units, requested_units);
    Ok(json!({ "line": line - 1, "character": requested_units }))
}

pub(super) fn lsp_range(
    source: &str,
    start_line: u32,
    start_column: u32,
    end_line: u32,
    end_column: u32,
) -> anyhow::Result<Value> {
    let start = lsp_position(source, start_line, start_column)?;
    let end = lsp_position(source, end_line, end_column)?;
    let start_key = (start_line, start_column);
    let end_key = (end_line, end_column);
    if start_key > end_key {
        anyhow::bail!("range start must not be after range end");
    }
    Ok(json!({ "start": start, "end": end }))
}
