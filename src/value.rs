use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use crate::{context::TycoContext, utils::unescape_basic_string};

#[derive(Clone, Debug)]
pub struct TycoString {
    pub value: String,
    pub has_template: bool,
    pub is_literal: bool,
}

impl TycoString {
    pub fn new(value: String, has_template: bool, is_literal: bool) -> Self {
        Self {
            value,
            has_template,
            is_literal,
        }
    }

    pub fn render(&mut self, ctx: &TycoContext, current: Option<&TycoInstance>) {
        if !self.has_template || self.is_literal {
            return;
        }

        let mut result = String::new();
        let mut chars = self.value.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut placeholder = String::new();
                while let Some(next) = chars.peek() {
                    if *next == '}' {
                        chars.next();
                        break;
                    }
                    placeholder.push(*next);
                    chars.next();
                }
                if let Some(resolved) = resolve_placeholder(&placeholder, ctx, current) {
                    result.push_str(&resolved);
                } else {
                    result.push('{');
                    result.push_str(&placeholder);
                    result.push('}');
                }
            } else {
                result.push(ch);
            }
        }

        self.value = unescape_basic_string(&result).unwrap_or(result);
        self.has_template = false;
    }
}

fn resolve_placeholder(
    placeholder: &str,
    ctx: &TycoContext,
    current: Option<&TycoInstance>,
) -> Option<String> {
    let path_parts: Vec<&str> = placeholder.split('.').collect();
    if path_parts.is_empty() {
        return None;
    }

    let mut segments = path_parts.as_slice();
    let mut from_global = false;
    if placeholder.starts_with("global.") && segments.len() > 1 {
        segments = &segments[1..];
        from_global = true;
    }

    let first = segments.first()?;
    let mut value = if from_global {
        ctx.globals().get(*first)
    } else if let Some(instance) = current {
        instance
            .get_attribute(first)
            .or_else(|| ctx.globals().get(*first))
    } else {
        ctx.globals().get(*first)
    }?;

    for segment in segments.iter().skip(1) {
        value = match value {
            TycoValue::Instance(instance) => instance.get_attribute(segment),
            TycoValue::Reference(reference) => reference
                .resolved
                .as_ref()
                .and_then(|inst| inst.get_attribute(segment)),
            _ => return None,
        }?;
    }

    Some(value.to_template_text())
}

#[derive(Clone, Debug)]
pub struct TycoInstance {
    struct_name: String,
    fields: IndexMap<String, TycoValue>,
    field_order: Vec<String>,
}

impl TycoInstance {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            struct_name: name.into(),
            fields: IndexMap::new(),
            field_order: Vec::new(),
        }
    }

    pub fn struct_name(&self) -> &str {
        &self.struct_name
    }

    pub fn set_attribute(&mut self, name: impl Into<String>, value: TycoValue) {
        let name = name.into();
        if !self.fields.contains_key(&name) {
            self.field_order.push(name.clone());
        }
        self.fields.insert(name, value);
    }

    pub fn get_attribute(&self, name: &str) -> Option<&TycoValue> {
        self.fields.get(name)
    }

    pub fn get_attribute_mut(&mut self, name: &str) -> Option<&mut TycoValue> {
        self.fields.get_mut(name)
    }

    pub fn remove_attribute(&mut self, name: &str) -> Option<TycoValue> {
        self.field_order.retain(|field| field != name);
        self.fields.shift_remove(name)
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.fields.contains_key(name)
    }

    pub fn rename_field(&mut self, from: &str, to: &str) {
        if let Some(value) = self.fields.shift_remove(from) {
            let mut replaced = false;
            for field in &mut self.field_order {
                if field == from {
                    *field = to.to_string();
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                self.field_order.push(to.to_string());
            }
            self.fields.insert(to.to_string(), value);
        }
    }

    pub fn attributes_mut(&mut self) -> &mut IndexMap<String, TycoValue> {
        &mut self.fields
    }

    pub fn attributes(&self) -> &IndexMap<String, TycoValue> {
        &self.fields
    }

    pub fn field_order(&self) -> &[String] {
        &self.field_order
    }

    pub fn enforce_order_from_schema(&mut self, schema: &[crate::context::FieldSchema]) {
        let mut ordered = Vec::new();
        for field in schema {
            if self.fields.contains_key(&field.name) {
                ordered.push(field.name.clone());
            }
        }
        for key in &self.field_order {
            if !ordered.contains(key) {
                ordered.push(key.clone());
            }
        }
        self.field_order = ordered;
    }
}

#[derive(Clone, Debug)]
pub struct TycoReference {
    pub struct_name: String,
    pub primary_key: String,
    pub resolved: Option<Box<TycoInstance>>,
}

impl TycoReference {
    pub fn new(struct_name: impl Into<String>, primary_key: impl Into<String>) -> Self {
        Self {
            struct_name: struct_name.into(),
            primary_key: primary_key.into(),
            resolved: None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum TycoValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(TycoString),
    Date(String),
    Time(String),
    DateTime(String),
    Array(Vec<TycoValue>),
    Instance(TycoInstance),
    Reference(TycoReference),
}

impl TycoValue {
    pub fn to_template_text(&self) -> String {
        match self {
            TycoValue::Null => "null".to_string(),
            TycoValue::Bool(v) => v.to_string(),
            TycoValue::Int(v) => v.to_string(),
            TycoValue::Float(v) => v.to_string(),
            TycoValue::String(s) => s.value.clone(),
            TycoValue::Date(v) | TycoValue::Time(v) | TycoValue::DateTime(v) => v.clone(),
            TycoValue::Array(_) => "[array]".to_string(),
            TycoValue::Instance(_) => "[instance]".to_string(),
            TycoValue::Reference(reference) => reference.primary_key.clone(),
        }
    }

    pub fn render_templates(&mut self, ctx: &TycoContext, current: Option<&TycoInstance>) {
        match self {
            TycoValue::String(s) => s.render(ctx, current),
            TycoValue::Array(items) => {
                for item in items {
                    item.render_templates(ctx, current);
                }
            }
            TycoValue::Instance(instance) => {
                let keys = instance.field_order().to_vec();
                let mut snapshot_instance = instance.clone();
                for key in keys {
                    if let Some(value) = instance.attributes_mut().get_mut(&key) {
                        value.render_templates(ctx, Some(&snapshot_instance));
                    }
                    snapshot_instance = instance.clone();
                }
            }
            _ => {}
        }
    }

    pub fn to_json_value(&self) -> JsonValue {
        match self {
            TycoValue::Null => JsonValue::Null,
            TycoValue::Bool(v) => JsonValue::Bool(*v),
            TycoValue::Int(v) => JsonValue::from(*v),
            TycoValue::Float(v) => JsonValue::from(*v),
            TycoValue::String(s) => JsonValue::from(s.value.clone()),
            TycoValue::Date(v) | TycoValue::Time(v) | TycoValue::DateTime(v) => {
                JsonValue::from(v.clone())
            }
            TycoValue::Array(items) => {
                JsonValue::Array(items.iter().map(|value| value.to_json_value()).collect())
            }
            TycoValue::Instance(instance) => {
                let mut map = serde_json::Map::new();
                for key in instance.field_order() {
                    if let Some(value) = instance.get_attribute(key) {
                        map.insert(key.clone(), value.to_json_value());
                    }
                }
                JsonValue::Object(map)
            }
            TycoValue::Reference(reference) => reference
                .resolved
                .as_ref()
                .map(|instance| TycoValue::Instance((**instance).clone()).to_json_value())
                .unwrap_or(JsonValue::Null),
        }
    }
}
