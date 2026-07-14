use serde::{
    de::{Error, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Number, Value};
use std::{collections::HashSet, fmt};

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct UniqueValueVisitor;

        impl<'de> Visitor<'de> for UniqueValueVisitor {
            type Value = UniqueValue;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("JSON value without duplicate object keys")
            }

            fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Number(Number::from(value))))
            }

            fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Number::from_f64(value)
                    .map(Value::Number)
                    .map(UniqueValue)
                    .ok_or_else(|| E::custom("JSON number must be finite"))
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Ok(UniqueValue(Value::String(value.to_string())))
            }

            fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }

            fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut values = Vec::new();
                while let Some(value) = sequence.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }

            fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut keys = HashSet::new();
                let mut values = Map::new();
                while let Some(key) = object.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(A::Error::custom(format!("duplicate JSON key: {key}")));
                    }
                    values.insert(key, object.next_value::<UniqueValue>()?.0);
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }

        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unit {
    pub pointer: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct Document {
    value: Value,
    units: Vec<Unit>,
}

impl Document {
    pub fn parse(source: &str) -> Option<Self> {
        let value = serde_json::from_str::<UniqueValue>(source).ok()?.0;
        if !value.is_object() && !value.is_array() {
            return None;
        }
        let mut units = Vec::new();
        collect_component(&value, "", &mut units);
        Some(Self { value, units })
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    pub fn render(&self, translations: &[(String, String)]) -> Result<String, String> {
        let mut value = self.value.clone();
        for (pointer, translation) in translations {
            let target = value
                .pointer_mut(pointer)
                .ok_or_else(|| format!("JSON 富文本回填路径不存在：{pointer}"))?;
            if !target.is_string() {
                return Err(format!("JSON 富文本回填目标不是字符串：{pointer}"));
            }
            *target = Value::String(translation.clone());
        }
        serde_json::to_string(&value).map_err(|e| format!("无法序列化 JSON 富文本：{e}"))
    }

    pub fn structure(&self) -> Value {
        let mut value = self.value.clone();
        for unit in &self.units {
            if let Some(target) = value.pointer_mut(&unit.pointer) {
                *target = Value::String("$display_text".into());
            }
        }
        value
    }

    pub fn text_at(&self, pointer: &str) -> Option<&str> {
        self.value.pointer(pointer)?.as_str()
    }
}

pub fn looks_like_component(source: &str) -> bool {
    let source = source.trim_start_matches('\u{feff}').trim_start();
    serde_json::from_str::<Value>(source).is_ok_and(|value| value.is_object() || value.is_array())
        || source.starts_with("{\"")
        || source.starts_with("[{")
        || source.starts_with("[\"")
}

fn push_string(value: &Value, pointer: String, units: &mut Vec<Unit>) {
    if let Some(source) = value.as_str().filter(|text| !text.trim().is_empty()) {
        units.push(Unit {
            pointer,
            source: source.to_string(),
        });
    }
}

fn collect_component(value: &Value, pointer: &str, units: &mut Vec<Unit>) {
    match value {
        Value::String(_) => push_string(value, pointer.to_string(), units),
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_component(child, &format!("{pointer}/{index}"), units);
            }
        }
        Value::Object(object) => {
            if let Some(text) = object.get("text") {
                push_string(text, format!("{pointer}/text"), units);
            }
            for key in ["extra", "with", "separator"] {
                if let Some(child) = object.get(key) {
                    collect_component(child, &format!("{pointer}/{key}"), units);
                }
            }
            if let Some(hover) = object.get("hoverEvent").and_then(Value::as_object) {
                match hover.get("action").and_then(Value::as_str) {
                    Some("show_text") => {
                        for key in ["contents", "value"] {
                            if let Some(child) = hover.get(key) {
                                collect_component(
                                    child,
                                    &format!("{pointer}/hoverEvent/{key}"),
                                    units,
                                );
                            }
                        }
                    }
                    Some("show_entity") => {
                        if let Some(name) = hover
                            .get("contents")
                            .and_then(Value::as_object)
                            .and_then(|contents| contents.get("name"))
                        {
                            collect_component(
                                name,
                                &format!("{pointer}/hoverEvent/contents/name"),
                                units,
                            );
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_player_facing_component_text() {
        let source = r#"{"text":"Open ","color":"gold","extra":[{"text":"guide","clickEvent":{"action":"open_url","value":"https://example.com"}},{"translate":"key.jump","with":["now"]}],"hoverEvent":{"action":"show_text","contents":{"text":"More info"}}}"#;
        let document = Document::parse(source).unwrap();
        assert_eq!(
            document.units(),
            [
                Unit {
                    pointer: "/text".into(),
                    source: "Open ".into()
                },
                Unit {
                    pointer: "/extra/0/text".into(),
                    source: "guide".into()
                },
                Unit {
                    pointer: "/extra/1/with/0".into(),
                    source: "now".into()
                },
                Unit {
                    pointer: "/hoverEvent/contents/text".into(),
                    source: "More info".into()
                }
            ]
        );
    }

    #[test]
    fn renders_translations_without_touching_component_structure() {
        let source = r#"{"text":"Open","clickEvent":{"action":"run_command","value":"/say hi"},"hoverEvent":{"action":"show_item","contents":{"id":"minecraft:stone"}}}"#;
        let document = Document::parse(source).unwrap();
        let rendered = document.render(&[("/text".into(), "打开".into())]).unwrap();
        let value: Value = serde_json::from_str(&rendered).unwrap();
        assert_eq!(value["text"], "打开");
        assert_eq!(value["clickEvent"]["value"], "/say hi");
        assert_eq!(value["hoverEvent"]["contents"]["id"], "minecraft:stone");
    }

    #[test]
    fn rejects_duplicate_json_keys_instead_of_silently_dropping_them() {
        let source = r#"{"text":"First","text":"Second"}"#;
        assert!(Document::parse(source).is_none());
        assert!(looks_like_component(source));
    }
}
