use super::*;

pub(super) fn read_config(
    path: &Path,
    default_content: &str,
    display_name: &str,
) -> Result<ConfigFile, String> {
    let content = if path.exists() {
        let content = fs::read_to_string(path)
            .map_err(|error| format!("无法读取 {display_name}：{error}"))?;
        if content.trim().is_empty() {
            default_content.to_string()
        } else {
            content
        }
    } else {
        default_content.to_string()
    };
    Ok(ConfigFile {
        path: path.display().to_string(),
        content,
    })
}

pub(super) fn validate_toml(content: &str) -> Vec<ConfigDiagnostic> {
    let Err(error) = toml::from_str::<toml::Value>(content) else {
        return Vec::new();
    };
    let span = error.span().unwrap_or(0..0);
    vec![ConfigDiagnostic {
        from: byte_offset_to_utf16(content, span.start),
        to: byte_offset_to_utf16(content, span.end),
        message: error.message().to_string(),
    }]
}

pub(super) fn format_toml(content: &str, filename: &str) -> Result<String, String> {
    parse_toml(content, filename)?;
    Ok(taplo::formatter::format(
        content,
        taplo::formatter::Options::default(),
    ))
}

pub(super) fn parse_toml(content: &str, filename: &str) -> Result<toml::Value, String> {
    toml::from_str(content).map_err(|error| format!("{filename} 格式错误：{error}"))
}

pub(super) fn validate_json_object(content: &str, display_name: &str) -> Vec<ConfigDiagnostic> {
    match serde_json::from_str::<Value>(content) {
        Ok(value) if value.is_object() => Vec::new(),
        Ok(_) => vec![ConfigDiagnostic {
            from: 0,
            to: content.encode_utf16().count().min(1),
            message: format!("{display_name} 必须是一个 JSON 对象。"),
        }],
        Err(error) => {
            let from = json_error_offset(content, &error);
            vec![ConfigDiagnostic {
                from,
                to: (from + 1).min(content.encode_utf16().count()),
                message: error.to_string(),
            }]
        }
    }
}

pub(super) fn format_json_object(content: &str, display_name: &str) -> Result<String, String> {
    serde_json::to_string_pretty(&parse_json_object(content, display_name)?)
        .map_err(|error| error.to_string())
}

pub(super) fn parse_json_object(content: &str, display_name: &str) -> Result<Value, String> {
    let value: Value = serde_json::from_str(content)
        .map_err(|error| format!("{display_name} 格式错误：{error}"))?;
    value
        .is_object()
        .then_some(value)
        .ok_or_else(|| format!("{display_name} 必须是一个 JSON 对象。"))
}

pub(super) fn byte_offset_to_utf16(content: &str, offset: usize) -> usize {
    content
        .char_indices()
        .take_while(|(index, _)| *index < offset)
        .map(|(_, character)| character.len_utf16())
        .sum()
}

fn json_error_offset(content: &str, error: &serde_json::Error) -> usize {
    let line_start = if error.line() <= 1 {
        0
    } else {
        content
            .match_indices('\n')
            .nth(error.line() - 2)
            .map_or(0, |(index, _)| index + 1)
    };
    let mut byte_offset = line_start
        .saturating_add(error.column().saturating_sub(1))
        .min(content.len());
    while !content.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }
    byte_offset_to_utf16(content, byte_offset)
}
