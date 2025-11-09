use std::collections::HashMap;

use std::borrow::Cow;

use indexmap::IndexMap;
use serde_json::Value as JsonValue;

use crate::{error::TycoError, value::TycoValue, value::TycoInstance};

#[derive(Clone, Debug)]
pub struct FieldSchema {
    pub name: String,
    pub type_name: String,
    pub is_primary_key: bool,
    pub is_nullable: bool,
    pub is_array: bool,
    pub default_value: Option<TycoValue>,
}

impl FieldSchema {
    pub fn new(name: impl Into<String>, type_name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            type_name: type_name.into(),
            is_primary_key: false,
            is_nullable: false,
            is_array: false,
            default_value: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TycoStruct {
    name: String,
    fields: Vec<FieldSchema>,
    primary_key_field: Option<String>,
    instances: Vec<TycoInstance>,
    primary_index: HashMap<String, TycoInstance>,
}

impl TycoStruct {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
            primary_key_field: None,
            instances: Vec::new(),
            primary_index: HashMap::new(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn fields(&self) -> &Vec<FieldSchema> {
        &self.fields
    }

    pub fn fields_mut(&mut self) -> &mut Vec<FieldSchema> {
        &mut self.fields
    }

    pub fn primary_key_field(&self) -> Option<&str> {
        self.primary_key_field.as_deref()
    }

    pub fn instances(&self) -> &Vec<TycoInstance> {
        &self.instances
    }

    pub fn instances_mut(&mut self) -> &mut Vec<TycoInstance> {
        &mut self.instances
    }

    pub fn add_field(&mut self, field: FieldSchema) {
        if field.is_primary_key {
            self.primary_key_field = Some(field.name.clone());
        }
        self.fields.push(field);
    }

    pub fn add_instance(&mut self, instance: TycoInstance) {
        self.instances.push(instance);
    }

    pub fn has_primary_key(&self) -> bool {
        self.primary_key_field.is_some()
    }

    pub fn set_default(
        &mut self,
        field_name: &str,
        value: Option<TycoValue>,
    ) -> Result<(), TycoError> {
        let field = self
            .fields
            .iter_mut()
            .find(|field| field.name == field_name)
            .ok_or_else(|| TycoError::parse(format!("Unknown field '{field_name}'")))?;
        field.default_value = value;
        Ok(())
    }

    pub fn build_primary_index(&mut self) -> Result<(), TycoError> {
        self.primary_index.clear();
        let Some(pk_field) = &self.primary_key_field else {
            return Ok(());
        };

        for instance in &self.instances {
            if let Some(value) = instance.get_attribute(pk_field) {
                self.primary_index
                    .insert(value.to_template_text(), instance.clone());
            }
        }
        Ok(())
    }

    pub fn find_by_primary_key(&self, key: &str) -> Option<&TycoInstance> {
        self.primary_index.get(key)
    }
}

#[derive(Clone, Debug)]
pub struct TycoContext {
    globals: IndexMap<String, TycoValue>,
    structs: IndexMap<String, TycoStruct>,
}

impl TycoContext {
    pub fn new() -> Self {
        Self {
            globals: IndexMap::new(),
            structs: IndexMap::new(),
        }
    }

    pub fn set_global(&mut self, name: impl Into<String>, value: TycoValue) {
        self.globals.insert(name.into(), value);
    }

    pub fn globals(&self) -> &IndexMap<String, TycoValue> {
        &self.globals
    }

    pub fn globals_mut(&mut self) -> &mut IndexMap<String, TycoValue> {
        &mut self.globals
    }

    pub fn add_struct(&mut self, tyco_struct: TycoStruct) {
        self.structs
            .entry(tyco_struct.name().to_string())
            .and_modify(|existing| {
                *existing = tyco_struct.clone();
            })
            .or_insert(tyco_struct);
    }

    pub fn structs(&self) -> &IndexMap<String, TycoStruct> {
        &self.structs
    }

    pub fn structs_mut(&mut self) -> &mut IndexMap<String, TycoStruct> {
        &mut self.structs
    }

    pub fn get_struct(&self, name: &str) -> Option<&TycoStruct> {
        self.structs.get(name)
    }

    pub fn get_struct_mut(&mut self, name: &str) -> Option<&mut TycoStruct> {
        self.structs.get_mut(name)
    }

    pub fn render(&mut self) -> Result<(), TycoError> {
        self.resolve_inline_instances()?;
        for struct_def in self.structs_mut().values_mut() {
            struct_def.build_primary_index()?;
        }
        self.resolve_references()?;
        self.render_templates();
        Ok(())
    }

    fn resolve_inline_instances(&mut self) -> Result<(), TycoError> {
        let schema_snapshot = self.structs.clone();

        fn coerce_value(value: TycoValue, schema: &FieldSchema) -> Result<TycoValue, TycoError> {
            if schema.is_array {
                return Ok(value);
            }
            match (schema.type_name.as_str(), value) {
                ("int", TycoValue::String(s)) => s
                    .value
                    .parse::<i64>()
                    .map(TycoValue::Int)
                    .map_err(|e| TycoError::parse(format!("Invalid int literal '{}': {e}", s.value))),
                ("float", TycoValue::String(s)) => s
                    .value
                    .parse::<f64>()
                    .map(TycoValue::Float)
                    .map_err(|e| TycoError::parse(format!("Invalid float literal '{}': {e}", s.value))),
                ("bool", TycoValue::String(s)) => Ok(TycoValue::Bool(matches!(
                    s.value.as_str(),
                    "true" | "True"
                ))),
                (_, other) => Ok(other),
            }
        }

        fn resolve_value(
            value: &mut TycoValue,
            schemas: &IndexMap<String, TycoStruct>,
        ) -> Result<(), TycoError> {
            match value {
                TycoValue::Array(items) => {
                    for item in items {
                        resolve_value(item, schemas)?;
                    }
                }
                TycoValue::Instance(instance) => {
                    if let Some(schema) = schemas.get(instance.struct_name()) {
                        apply_schema(instance, schema, schemas)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }

        fn apply_schema(
            instance: &mut TycoInstance,
            schema: &TycoStruct,
            schemas: &IndexMap<String, TycoStruct>,
        ) -> Result<(), TycoError> {
            let mut positional = Vec::new();
            for key in instance.field_order() {
                if let Some(idx) = key.strip_prefix("_arg").and_then(|rest| rest.parse::<usize>().ok()) {
                    positional.push((idx, key.clone()));
                }
            }
            positional.sort_by_key(|(idx, _)| *idx);

            for (idx, placeholder) in positional {
                if let Some(field_schema) = schema.fields().get(idx) {
                    if let Some(value) = instance.remove_attribute(&placeholder) {
                        let coerced = coerce_value(value, field_schema)?;
                        instance.set_attribute(field_schema.name.clone(), coerced);
                    }
                }
            }

            for field in schema.fields() {
                if let Some(value) = instance.remove_attribute(&field.name) {
                    let coerced = coerce_value(value, field)?;
                    instance.set_attribute(field.name.clone(), coerced);
                } else if let Some(default) = &field.default_value {
                    instance.set_attribute(field.name.clone(), default.clone());
                }
            }

            instance.enforce_order_from_schema(schema.fields());

            for value in instance.attributes_mut().values_mut() {
                resolve_value(value, schemas)?;
            }

            Ok(())
        }

        let global_keys = self.globals.keys().cloned().collect::<Vec<_>>();
        for key in global_keys {
            if let Some(value) = self.globals.get_mut(&key) {
                resolve_value(value, &schema_snapshot)?;
            }
        }

        for (name, struct_def) in self.structs.iter_mut() {
            let schema_cow = schema_snapshot
                .get(name)
                .map(Cow::Borrowed)
                .unwrap_or_else(|| Cow::Owned(struct_def.clone()));
            for instance in struct_def.instances_mut() {
                apply_schema(instance, schema_cow.as_ref(), &schema_snapshot)?;
            }
        }

        Ok(())
    }

    fn resolve_references(&mut self) -> Result<(), TycoError> {
        let struct_snapshot = self.structs.clone();

        fn visit(
            value: &mut TycoValue,
            structs: &IndexMap<String, TycoStruct>,
        ) -> Result<(), TycoError> {
            match value {
                TycoValue::Reference(reference) => {
                    let struct_def = structs
                        .get(&reference.struct_name)
                        .ok_or_else(|| TycoError::UnknownStruct(reference.struct_name.clone()))?;
                    let pk = struct_def
                        .find_by_primary_key(&reference.primary_key)
                        .cloned()
                        .ok_or_else(|| {
                            TycoError::Reference(format!(
                                "Unknown {}({})",
                                reference.struct_name, reference.primary_key
                            ))
                        })?;
                    reference.resolved = Some(Box::new(pk));
                }
                TycoValue::Array(items) => {
                    for item in items {
                        visit(item, structs)?;
                    }
                }
                TycoValue::Instance(instance) => {
                    for value in instance.attributes_mut().values_mut() {
                        visit(value, structs)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }

        let global_keys = self.globals.keys().cloned().collect::<Vec<_>>();
        for key in global_keys {
            if let Some(value) = self.globals.get_mut(&key) {
                visit(value, &struct_snapshot)?;
            }
        }

        for struct_def in self.structs.values_mut() {
            for instance in struct_def.instances_mut() {
                for value in instance.attributes_mut().values_mut() {
                    visit(value, &struct_snapshot)?;
                }
            }
        }
        Ok(())
    }

    fn render_templates(&mut self) {
        let mut snapshot = self.clone();
        let global_keys = self.globals.keys().cloned().collect::<Vec<_>>();
        for key in global_keys {
            if let Some(value) = self.globals.get_mut(&key) {
                value.render_templates(&snapshot, None);
                snapshot
                    .globals_mut()
                    .insert(key.clone(), value.clone());
            }
        }

        let struct_names = self.structs.keys().cloned().collect::<Vec<_>>();
        for name in struct_names {
            let Some(struct_def) = self.structs.get_mut(&name) else {
                continue;
            };
            for (idx, instance) in struct_def.instances_mut().iter_mut().enumerate() {
                let keys = instance.field_order().to_vec();
                let mut instance_snapshot = instance.clone();
                for key in keys {
                    if let Some(value) = instance.attributes_mut().get_mut(&key) {
                        value.render_templates(&snapshot, Some(&instance_snapshot));
                    }
                    instance_snapshot = instance.clone();
                }
                if let Some(snapshot_struct) = snapshot.get_struct_mut(&name) {
                    if let Some(slot) = snapshot_struct.instances_mut().get_mut(idx) {
                        *slot = instance.clone();
                    } else {
                        snapshot_struct.instances_mut().push(instance.clone());
                    }
                }
            }
        }
    }

    pub fn to_json(&self) -> JsonValue {
        let mut map = serde_json::Map::new();
        for (name, value) in self.globals.iter() {
            map.insert(name.clone(), value.to_json_value());
        }
        for (name, struct_def) in self.structs.iter() {
            if struct_def.primary_key_field().is_some() {
                let instances = struct_def
                    .instances()
                    .iter()
                    .map(|instance| {
                        let mut obj = serde_json::Map::new();
                        for key in instance.field_order() {
                            if let Some(value) = instance.get_attribute(key) {
                                obj.insert(key.clone(), value.to_json_value());
                            }
                        }
                        JsonValue::Object(obj)
                    })
                    .collect::<Vec<_>>();
                map.insert(name.clone(), JsonValue::Array(instances));
            }
        }
        JsonValue::Object(map)
    }
}
