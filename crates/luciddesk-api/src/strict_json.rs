//! Reject duplicate keys at every depth before decoding plans or shorthand fields.
use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use std::{collections::HashSet, fmt};
struct Checked;
impl<'de> Deserialize<'de> for Checked {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Check;
        impl<'de> Visitor<'de> for Check {
            type Value = Checked;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Checked, E> {
                Ok(Checked)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Checked, A::Error> {
                while seq.next_element::<Checked>()?.is_some() {}
                Ok(Checked)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Checked, A::Error> {
                let mut keys = HashSet::new();
                while let Some(key) = map.next_key::<String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(de::Error::custom(format!("duplicate JSON key: {key}")));
                    }
                    map.next_value::<Checked>()?;
                }
                Ok(Checked)
            }
        }
        deserializer.deserialize_any(Check)
    }
}
/// Validates bounded protocol JSON without accepting duplicate object keys.
/// # Errors
/// Returns a JSON error for duplicate keys, malformed input or excessive nesting.
pub fn validate_json(bytes: &[u8]) -> Result<(), serde_json::Error> {
    serde_json::from_slice::<Checked>(bytes).map(|_| ())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_fields_in_nested_updates_are_rejected() {
        assert!(validate_json(br#"{"values":{"enabled":true,"enabled":false}}"#).is_err());
        assert!(validate_json(br#"{"a":1,"\u0061":2}"#).is_err());
        assert!(validate_json(br#"[{"a":1},{"a":2}]"#).is_ok());
        assert!(validate_json(br#"{"a":null,"b":[true,2,"text"]}"#).is_ok());
    }
}
