use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::{
    context::{FieldSchema, TycoContext, TycoStruct},
    error::TycoError,
    utils::{
        has_unclosed_delimiter, normalize_datetime, normalize_time, parse_integer, split_top_level,
        strip_inline_comment, strip_leading_newline, unescape_basic_string,
    },
    value::{TycoInstance, TycoReference, TycoString, TycoValue},
};

static STRUCT_DEF_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([A-Z][A-Za-z0-9_]*)\s*:$").unwrap());
static FIELD_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"^\s*([*?])?([A-Za-z][A-Za-z0-9_]*)(\[\])?\s+([a-z_][A-Za-z0-9_]*)\s*:(?:\s+(.*))?$",
    )
    .unwrap()
});
static DEFAULT_UPDATE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s+([a-z_][A-Za-z0-9_]*)\s*:(?:\s+(.*))?$").unwrap());
static STRUCT_CALL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^([A-Za-z][A-Za-z0-9_]*)\((.*)\)$").unwrap());

#[derive(Copy, Clone, Eq, PartialEq)]
enum ParseState {
    TopLevel,
    InStructSchema,
    InStructInstances,
}

pub struct TycoParser {
    included: HashSet<PathBuf>,
}

impl TycoParser {
    pub fn new() -> Self {
        Self {
            included: HashSet::new(),
        }
    }

    fn is_valid_field_name(name: &str) -> bool {
        let mut chars = name.chars();
        match chars.next() {
            Some(first) if first.is_ascii_alphabetic() || first == '_' => {
                chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            }
            _ => false,
        }
    }

    pub fn parse_file<P: AsRef<Path>>(&mut self, path: P) -> Result<TycoContext, TycoError> {
        let lines = self.read_file_with_includes(path.as_ref())?;
        self.parse_lines(&lines)
    }

    pub fn parse_str(&mut self, content: &str) -> Result<TycoContext, TycoError> {
        let lines = content
            .lines()
            .map(|line| line.to_string())
            .collect::<Vec<_>>();
        self.parse_lines(&lines)
    }

    fn read_file_with_includes(&mut self, path: &Path) -> Result<Vec<String>, TycoError> {
        let canonical = fs::canonicalize(path)?;
        if !self.included.insert(canonical.clone()) {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&canonical)?;
        let mut result = Vec::new();
        let parent = canonical
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        for line in content.lines() {
            if let Some(include_path) = line.trim().strip_prefix("#include") {
                let include = include_path.trim().trim_matches(['"', '\''].as_ref());
                let include_full = parent.join(include);
                let nested = self.read_file_with_includes(&include_full)?;
                result.extend(nested);
            } else {
                result.push(line.to_string());
            }
        }
        Ok(result)
    }

    fn parse_lines(&mut self, lines: &[String]) -> Result<TycoContext, TycoError> {
        let mut context = TycoContext::new();
        let mut state = ParseState::TopLevel;
        let mut current_struct: Option<String> = None;
        let mut instance_lines: Vec<String> = Vec::new();

        let mut idx = 0;
        while idx < lines.len() {
            let line = &lines[idx];
            let trimmed = strip_inline_comment(line);
            let trimmed_ws = trimmed.trim();

            if trimmed_ws.is_empty() {
                idx += 1;
                continue;
            }

            if let Some(caps) = STRUCT_DEF_RE.captures(trimmed_ws) {
                if let (Some(struct_name), ParseState::InStructInstances) =
                    (&current_struct, state)
                {
                    self.parse_struct_instances(struct_name, &instance_lines, &mut context)?;
                    instance_lines.clear();
                }
                current_struct = Some(caps[1].to_string());
                if context.get_struct(&caps[1]).is_none() {
                    context.add_struct(TycoStruct::new(&caps[1]));
                }
                state = ParseState::InStructSchema;
                idx += 1;
                continue;
            }

            if let Some(caps) = FIELD_RE.captures(line) {
                let is_primary = caps.get(1).map_or(false, |m| m.as_str() == "*");
                let is_nullable = caps.get(1).map_or(false, |m| m.as_str() == "?");
                let type_name = caps[2].to_string();
                let is_array = caps.get(3).is_some();
                let attr_name = caps[4].to_string();
                let mut value_str = caps.get(5).map(|m| m.as_str().to_string()).unwrap_or_default();

                if has_unclosed_delimiter(&value_str, "\"\"\"")
                    || has_unclosed_delimiter(&value_str, "'''")
                {
                    let delimiter = if value_str.contains("\"\"\"") {
                        "\"\"\""
                    } else {
                        "'''"
                    };
                    idx = Self::accumulate_multiline(idx, lines, &mut value_str, delimiter);
                }

                value_str = strip_inline_comment(&value_str);

                let is_global_line = line.chars().next().map_or(false, |c| !c.is_whitespace());
                if !is_global_line && current_struct.is_none() {
                    return Err(TycoError::parse("Struct field defined before struct header"));
                }

                if !is_global_line {
                    let struct_name = current_struct.as_ref().unwrap().clone();
                    let mut field = FieldSchema::new(&attr_name, &type_name);
                    field.is_primary_key = is_primary;
                    field.is_nullable = is_nullable;
                    field.is_array = is_array;
                    if !value_str.is_empty() {
                        let ty = field_type_name(&field);
                        let parsed = self.parse_value(&value_str, &ty, &context)?;
                        field.default_value = Some(parsed);
                    }
                    context
                        .get_struct_mut(&struct_name)
                        .ok_or_else(|| TycoError::UnknownStruct(struct_name.clone()))?
                        .add_field(field);
                    state = ParseState::InStructSchema;
                } else {
                    let type_descriptor = field_type_descriptor(&type_name, is_array);
                    let value =
                        self.parse_value(&value_str, &type_descriptor, &context)?;
                    context.set_global(attr_name, value);
                    state = ParseState::TopLevel;
                }
                idx += 1;
                continue;
            }

            if let Some(caps) = DEFAULT_UPDATE_RE.captures(line) {
                if let Some(struct_name) = &current_struct {
                    let field_name = caps[1].to_string();
                    let mut value_str = caps.get(2).map(|m| m.as_str().to_string()).unwrap_or_default();
                    if has_unclosed_delimiter(&value_str, "\"\"\"")
                        || has_unclosed_delimiter(&value_str, "'''")
                    {
                        let delimiter = if value_str.contains("\"\"\"") {
                            "\"\"\""
                        } else {
                            "'''"
                        };
                        idx = Self::accumulate_multiline(idx, lines, &mut value_str, delimiter);
                    }

                    value_str = strip_inline_comment(&value_str);
                    let parsed_value = if value_str.trim().is_empty() {
                        None
                    } else {
                        let schema = context
                            .get_struct(struct_name)
                            .ok_or_else(|| TycoError::UnknownStruct(struct_name.clone()))?;
                        let field_schema = schema
                            .fields()
                            .iter()
                            .find(|field| field.name == field_name)
                            .ok_or_else(|| TycoError::parse(format!("Unknown field '{field_name}'")))?;
                        let ty = field_type_name(field_schema);
                        Some(self.parse_value(&value_str, &ty, &context)?)
                    };

                    context
                        .get_struct_mut(struct_name)
                        .ok_or_else(|| TycoError::UnknownStruct(struct_name.clone()))?
                        .set_default(&field_name, parsed_value)?;
                    idx += 1;
                    continue;
                }
            }

            if trimmed_ws.starts_with('-') {
                if current_struct.is_none() {
                    return Err(TycoError::parse(
                        "Instance data encountered outside of a struct block",
                    ));
                }
                state = ParseState::InStructInstances;
                let mut inst_line = trimmed_ws.trim_start_matches('-').trim().to_string();
                while inst_line.ends_with('\\') && idx + 1 < lines.len() {
                    inst_line.pop();
                    idx += 1;
                    inst_line.push(' ');
                    inst_line.push_str(strip_inline_comment(&lines[idx]).trim());
                }
                if has_unclosed_delimiter(&inst_line, "\"\"\"") || has_unclosed_delimiter(&inst_line, "'''") {
                    let delimiter = if inst_line.contains("\"\"\"") {
                        "\"\"\""
                    } else {
                        "'''"
                    };
                    idx = Self::accumulate_multiline(idx, lines, &mut inst_line, delimiter);
                }
                instance_lines.push(inst_line);
                idx += 1;
                continue;
            }

            if state == ParseState::InStructInstances
                && line.chars().next().map_or(false, |c| c.is_whitespace())
            {
                if let Some(last) = instance_lines.last_mut() {
                    last.push(' ');
                    last.push_str(trimmed_ws);
                }
                idx += 1;
                continue;
            }

            idx += 1;
        }

        if let Some(struct_name) = &current_struct {
            if !instance_lines.is_empty() {
                self.parse_struct_instances(struct_name, &instance_lines, &mut context)?;
            }
        }

        context.render()?;
        Ok(context)
    }

    fn accumulate_multiline(
        idx: usize,
        lines: &[String],
        value_str: &mut String,
        delimiter: &str,
    ) -> usize {
        let mut cursor = idx;
        while cursor + 1 < lines.len() && has_unclosed_delimiter(value_str, delimiter) {
            cursor += 1;
            value_str.push('\n');
            value_str.push_str(&lines[cursor]);
            if !has_unclosed_delimiter(value_str, delimiter) {
                break;
            }
        }
        cursor
    }

    fn parse_struct_instances(
        &self,
        struct_name: &str,
        instance_lines: &[String],
        context: &mut TycoContext,
    ) -> Result<(), TycoError> {
        if instance_lines.is_empty() {
            return Ok(());
        }

        let fields = context
            .get_struct(struct_name)
            .ok_or_else(|| TycoError::UnknownStruct(struct_name.to_string()))?
            .fields()
            .to_vec();

        for line in instance_lines {
            let parts = split_top_level(line, ',');
            let mut instance = TycoInstance::new(struct_name);
            let mut positional_index = 0;
            let mut using_named = false;
            for part in parts {
                let part = part.trim();
                if part.is_empty() {
                    continue;
                }
                if let Some((field, value)) = Self::split_named_argument(&part) {
                    using_named = true;
                    let schema = fields
                        .iter()
                        .find(|f| f.name == field)
                        .ok_or_else(|| TycoError::parse(format!("Unknown field '{field}' in {struct_name}")))?;
                    let ty = field_type_name(schema);
                    let typed_value = self.parse_value(value.trim(), &ty, context)?;
                    instance.set_attribute(field.to_string(), typed_value);
                } else {
                    if using_named {
                        return Err(TycoError::parse(
                            "Positional arguments cannot follow named arguments",
                        ));
                    }
                    if positional_index >= fields.len() {
                        return Err(TycoError::parse(format!(
                            "Too many positional arguments for {struct_name}"
                        )));
                    }
                    let schema = &fields[positional_index];
                    let ty = field_type_name(schema);
                    let typed_value = self.parse_value(part, &ty, context)?;
                    instance.set_attribute(schema.name.clone(), typed_value);
                    positional_index += 1;
                }
            }

            let struct_mut = context
                .get_struct_mut(struct_name)
                .ok_or_else(|| TycoError::UnknownStruct(struct_name.to_string()))?;
            struct_mut.add_instance(instance);
        }

        Ok(())
    }

    fn split_named_argument(part: &str) -> Option<(&str, &str)> {
        let mut depth: i32 = 0;
        let mut in_quotes = false;
        let mut quote_char = '\0';
        let chars: Vec<char> = part.chars().collect();
        let mut idx = 0;
        while idx < chars.len() {
            let ch = chars[idx];
            if in_quotes {
                if ch == quote_char {
                    in_quotes = false;
                } else if ch == '\\' {
                    idx += 1; // skip escaped char
                }
            } else {
                match ch {
                    '"' | '\'' => {
                        in_quotes = true;
                        quote_char = ch;
                    }
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth = depth.saturating_sub(1),
                    ':' if depth == 0 => {
                        let name = part[..idx].trim();
                        let value = part[idx + 1..].trim();
                        if Self::is_valid_field_name(name) && !value.is_empty() {
                            return Some((name, value));
                        }
                    }
                    _ => {}
                }
            }
            idx += 1;
        }
        None
    }

    fn parse_value(
        &self,
        token: &str,
        type_name: &str,
        context: &TycoContext,
    ) -> Result<TycoValue, TycoError> {
        let trimmed = token.trim();
        if trimmed.eq_ignore_ascii_case("null") {
            return Ok(TycoValue::Null);
        }
        match type_name {
            "bool" => {
                if trimmed == "true" {
                    Ok(TycoValue::Bool(true))
                } else if trimmed == "false" {
                    Ok(TycoValue::Bool(false))
                } else {
                    Err(TycoError::parse(format!("Invalid bool literal '{trimmed}'")))
                }
            }
            "int" => Ok(TycoValue::Int(parse_integer(trimmed)?)),
            "float" => {
                let value = trimmed.parse::<f64>().map_err(|e| {
                    TycoError::parse(format!("Invalid float literal '{trimmed}': {e}"))
                })?;
                Ok(TycoValue::Float(value))
            }
            "date" => Ok(TycoValue::Date(parse_string_value(trimmed)?.value)),
            "time" => Ok(TycoValue::Time(normalize_time(&parse_string_value(trimmed)?.value))),
            "datetime" => Ok(TycoValue::DateTime(normalize_datetime(
                &parse_string_value(trimmed)?.value,
            ))),
            "str" => Ok(TycoValue::String(parse_string_value(trimmed)?)),
            _ if type_name.ends_with("[]") => {
                let base = &type_name[..type_name.len() - 2];
                if trimmed == "[]" {
                    return Ok(TycoValue::Array(Vec::new()));
                }
                if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
                    return Err(TycoError::parse(format!(
                        "Array literal must be wrapped in []: {trimmed}"
                    )));
                }
                let inner = &trimmed[1..trimmed.len() - 1];
                let items = split_top_level(inner, ',');
                let mut values = Vec::new();
                for item in items {
                    if item.trim().is_empty() {
                        continue;
                    }
                    values.push(self.parse_value(item.trim(), base, context)?);
                }
                Ok(TycoValue::Array(values))
            }
            _ => self.parse_struct_call(trimmed, type_name, context),
        }
    }

    fn parse_struct_call(
        &self,
        token: &str,
        type_name: &str,
        context: &TycoContext,
    ) -> Result<TycoValue, TycoError> {
        if let Some(caps) = STRUCT_CALL_RE.captures(token) {
            let struct_name = caps[1].to_string();
            let args = caps[2].to_string();
            match context.get_struct(&struct_name) {
                Some(def) if def.has_primary_key() => {
                    let pk = parse_string_value(args.trim())?.value;
                    return Ok(TycoValue::Reference(TycoReference::new(struct_name, pk)));
                }
                Some(_) => {
                    let inline_instance = self.parse_inline_instance(&struct_name, &args)?;
                    return Ok(TycoValue::Instance(inline_instance));
                }
                None => {
                    let pk = parse_string_value(args.trim())?.value;
                    return Ok(TycoValue::Reference(TycoReference::new(struct_name, pk)));
                }
            }
        }
        Err(TycoError::parse(format!(
            "Cannot parse value '{token}' as type '{type_name}'"
        )))
    }

    fn parse_inline_instance(
        &self,
        struct_name: &str,
        args_str: &str,
    ) -> Result<TycoInstance, TycoError> {
        let mut instance = TycoInstance::new(struct_name);
        let parts = split_top_level(args_str, ',');
        let mut position = 0;
        for part in parts {
            if let Some((name, value)) = Self::split_named_argument(&part) {
                let parsed = parse_string_value(value.trim())?;
                instance.set_attribute(name.to_string(), TycoValue::String(parsed));
            } else {
                let parsed = parse_string_value(part.trim())?;
                instance.set_attribute(format!("_arg{position}"), TycoValue::String(parsed));
                position += 1;
            }
        }
        Ok(instance)
    }
}

impl Default for TycoParser {
    fn default() -> Self {
        Self::new()
    }
}

pub fn load<P: AsRef<Path>>(path: P) -> Result<TycoContext, TycoError> {
    TycoParser::new().parse_file(path)
}

pub fn loads(content: &str) -> Result<TycoContext, TycoError> {
    TycoParser::new().parse_str(content)
}

fn field_type_name(field: &FieldSchema) -> String {
    field_type_descriptor(&field.type_name, field.is_array)
}

fn field_type_descriptor(base: &str, is_array: bool) -> String {
    if is_array {
        format!("{base}[]")
    } else {
        base.to_string()
    }
}

fn parse_string_value(token: &str) -> Result<TycoString, TycoError> {
    if token.starts_with("\"\"\"") {
        if let Some(end) = token[3..].find("\"\"\"") {
            let raw = &token[3..3 + end];
            let content = strip_leading_newline(raw);
            let content = unescape_basic_string(&content)?;
            let has_template = content.contains('{') && content.contains('}');
            return Ok(TycoString::new(content, has_template, false));
        }
    }
    if token.starts_with("'''") {
        if let Some(end) = token[3..].find("'''") {
            let content = token[3..3 + end].to_string();
            return Ok(TycoString::new(content, false, true));
        }
    }
    if token.starts_with('"') && token.ends_with('"') {
        let inner = &token[1..token.len() - 1];
        let content = unescape_basic_string(inner)?;
        let has_template = content.contains('{') && content.contains('}');
        return Ok(TycoString::new(content, has_template, false));
    }
    if token.starts_with('\'') && token.ends_with('\'') {
        let inner = &token[1..token.len() - 1];
        return Ok(TycoString::new(inner.to_string(), false, true));
    }
    Ok(TycoString::new(
        token.to_string(),
        token.contains('{') && token.contains('}'),
        false,
    ))
}
