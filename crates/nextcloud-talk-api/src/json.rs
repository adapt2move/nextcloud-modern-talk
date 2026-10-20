//! Shared bounded JSON ingress. Unknown fields survive until typed decoding; duplicate keys fail.
use crate::error::ErrorKind;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::fmt;
/// Decode with a shared structure budget. Keys, values and containers all count before
/// insertion; duplicate keys fail instead of silently overriding a restriction.
pub(crate) fn json(bytes: &[u8]) -> Result<Value, ErrorKind> {
    if bytes.len() > super::transport::BODY_LIMIT {
        return Err(ErrorKind::TooLarge);
    }
    let mut budget = 4096usize;
    let mut de = serde_json::Deserializer::from_slice(bytes);
    let value = Seed {
        budget: &mut budget,
        depth: 0,
    }
    .deserialize(&mut de)
    .map_err(|_| ErrorKind::Malformed)?;
    de.end().map_err(|_| ErrorKind::Malformed)?;
    Ok(value)
}
struct Seed<'a> {
    budget: &'a mut usize,
    depth: u8,
}
impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        if *self.budget == 0 || self.depth > 64 {
            return Err(de::Error::custom("structure limit"));
        }
        *self.budget -= 1;
        de.deserialize_any(self)
    }
}
impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("bounded JSON")
    }
    fn visit_bool<E: de::Error>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Value, E> {
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v)
            .map(Value::Number)
            .ok_or_else(|| E::custom("number"))
    }
    fn visit_str<E: de::Error>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_owned()))
    }
    fn visit_string<E: de::Error>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_none<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_unit<E: de::Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(v) = seq.next_element_seed(Seed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            values.push(v);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if *self.budget == 0 || values.contains_key(&key) {
                return Err(de::Error::custom("duplicate or limit"));
            }
            *self.budget -= 1;
            let value = map.next_value_seed(Seed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hostile_json_has_finite_structure_and_no_duplicate_policy_keys() {
        assert!(json(br#"{"permission":true,"permission":false}"#).is_err());
        assert!(json(format!("{}0{}", "[".repeat(66), "]".repeat(66)).as_bytes()).is_err());
        assert!(json(format!("[{}0]", "0,".repeat(4097)).as_bytes()).is_err());
        assert!(json(b"{} trailing").is_err());
        assert!(json(br#"{"newField":{"unknown":true},"text":"Unicode \u00fc"}"#).is_ok());
    }
}
