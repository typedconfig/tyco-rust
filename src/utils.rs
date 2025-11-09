use std::borrow::Cow;

use crate::error::TycoError;

pub fn strip_inline_comment(line: &str) -> String {
    let mut in_quotes = false;
    let mut quote_char = '\0';

    for (idx, ch) in line.chars().enumerate() {
        if !in_quotes && (ch == '"' || ch == '\'') {
            in_quotes = true;
            quote_char = ch;
        } else if in_quotes && ch == quote_char && line[..idx].chars().last().unwrap_or('\0') != '\\' {
            in_quotes = false;
            quote_char = '\0';
        } else if ch == '#' && !in_quotes {
            return line[..idx].trim_end().to_string();
        }
    }

    line.trim_end().to_string()
}

pub fn has_unclosed_delimiter(line: &str, delimiter: &str) -> bool {
    if let Some(start) = line.find(delimiter) {
        line[start + delimiter.len()..].find(delimiter).is_none()
    } else {
        false
    }
}

pub fn split_top_level(input: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;
    let mut in_quotes = false;
    let mut quote_char = '\0';
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == quote_char && !is_escaped(&current) {
                in_quotes = false;
                quote_char = '\0';
            }
            current.push(ch);
            continue;
        }

        match ch {
            '"' | '\'' => {
                in_quotes = true;
                quote_char = ch;
                current.push(ch);
            }
            '[' | '{' | '(' => {
                depth += 1;
                current.push(ch);
            }
            ']' | '}' | ')' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            c if c == delimiter && depth == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

fn is_escaped(current: &str) -> bool {
    let mut backslashes = 0;
    for ch in current.chars().rev() {
        if ch == '\\' {
            backslashes += 1;
        } else {
            break;
        }
    }
    backslashes % 2 == 1
}

pub fn parse_integer(token: &str) -> Result<i64, TycoError> {
    let trimmed = token.trim();
    let negative = trimmed.starts_with('-');
    let body = if negative { &trimmed[1..] } else { trimmed };

    let value = if body.starts_with("0x") || body.starts_with("0X") {
        i64::from_str_radix(&body[2..], 16)
    } else if body.starts_with("0o") || body.starts_with("0O") {
        i64::from_str_radix(&body[2..], 8)
    } else if body.starts_with("0b") || body.starts_with("0B") {
        i64::from_str_radix(&body[2..], 2)
    } else {
        body.parse()
    }
    .map_err(|e| TycoError::parse(format!("Failed to parse integer '{token}': {e}")))?;

    Ok(if negative { -value } else { value })
}

pub fn normalize_time(value: &str) -> String {
    if let Some(idx) = value.find('.') {
        let (head, tail) = value.split_at(idx + 1);
        let mut fractional = tail.to_string();
        fractional.truncate(fractional.find(|c: char| !c.is_ascii_digit()).unwrap_or(fractional.len()));
        if fractional.len() < 6 {
            fractional.push_str(&"0".repeat(6 - fractional.len()));
        } else if fractional.len() > 6 {
            fractional.truncate(6);
        }
        format!("{head}{fractional}")
    } else {
        value.to_string()
    }
}

pub fn normalize_datetime(value: &str) -> String {
    let mut result = value.replace(' ', "T");
    if result.ends_with('Z') {
        result.pop();
        result.push_str("+00:00");
    }
    if let Some(idx) = result.find('.') {
        let tz_start = result[idx..]
            .find(|c: char| c == '+' || c == '-')
            .map(|offset| idx + offset)
            .unwrap_or(result.len());
        let fractional = normalize_time(&result[idx..tz_start]);
        format!("{}{}{}", &result[..idx], fractional, &result[tz_start..])
    } else {
        result
    }
}

pub fn unescape_basic_string(value: &str) -> Result<String, TycoError> {
    let mut chars = value.chars().peekable();
    let mut output = String::with_capacity(value.len());

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some('r') => output.push('\r'),
            Some('b') => output.push('\u{0008}'),
            Some('f') => output.push('\u{000C}'),
            Some('"') => output.push('"'),
            Some('\\') => output.push('\\'),
            Some('u') => {
                let code = take_hex(&mut chars, 4)?;
                let value = u32::from_str_radix(&code, 16)
                    .map_err(|_| TycoError::parse(format!("Invalid unicode escape: \\u{code}")))?;
                output.push(
                    char::from_u32(value)
                        .ok_or_else(|| TycoError::parse(format!("Invalid unicode codepoint: {code}")))?,
                );
            }
            Some('U') => {
                let code = take_hex(&mut chars, 8)?;
                let value = u32::from_str_radix(&code, 16)
                    .map_err(|_| TycoError::parse(format!("Invalid unicode escape: \\U{code}")))?;
                output.push(
                    char::from_u32(value)
                        .ok_or_else(|| TycoError::parse(format!("Invalid unicode codepoint: {code}")))?,
                );
            }
            Some(other) => {
                output.push('\\');
                output.push(other);
            }
            None => output.push('\\'),
        }
    }

    Ok(output)
}

fn take_hex(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    len: usize,
) -> Result<String, TycoError> {
    let mut buf = String::new();
    for _ in 0..len {
        let Some(ch) = chars.next() else {
            return Err(TycoError::parse("Incomplete unicode escape"));
        };
        buf.push(ch);
    }
    Ok(buf)
}

pub fn strip_leading_newline(value: &str) -> Cow<'_, str> {
    if value.starts_with('\n') {
        Cow::Owned(value[1..].to_string())
    } else {
        Cow::Borrowed(value)
    }
}
