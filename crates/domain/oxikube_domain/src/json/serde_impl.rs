//! `serde` for [`JsonRef`] and [`JsonDoc`]: a view serialises as the JSON it stands for, without
//! building a [`Value`]; a document deserialises from any JSON.

use serde::de::{Deserialize, Deserializer};
use serde::ser::{Error as _, Serialize, SerializeMap, SerializeSeq, Serializer};
use serde_json::Value;

use super::JsonDoc;
use super::reader::{JsonKind, JsonRef};

impl Serialize for JsonRef<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.kind() {
            JsonKind::Null => serializer.serialize_unit(),
            JsonKind::Bool => serializer.serialize_bool(self.as_bool().unwrap_or(false)),
            JsonKind::Number => {
                if let Some(n) = self.as_i64() {
                    serializer.serialize_i64(n)
                } else if let Some(n) = self.as_u64() {
                    serializer.serialize_u64(n)
                } else if let Some(n) = self.as_f64() {
                    serializer.serialize_f64(n)
                } else {
                    Err(S::Error::custom("corrupt number"))
                }
            }
            JsonKind::String => serializer.serialize_str(self.as_str().unwrap_or_default()),
            JsonKind::Array => {
                let array = self
                    .as_array()
                    .ok_or_else(|| S::Error::custom("corrupt array"))?;
                let mut seq = serializer.serialize_seq(Some(array.len()))?;
                for item in array.iter() {
                    seq.serialize_element(&item)?;
                }
                seq.end()
            }
            JsonKind::Object => {
                let object = self
                    .as_object()
                    .ok_or_else(|| S::Error::custom("corrupt object"))?;
                let mut map = serializer.serialize_map(Some(object.len()))?;
                for (key, value) in object.iter() {
                    map.serialize_entry(key, &value)?;
                }
                map.end()
            }
        }
    }
}

impl Serialize for JsonDoc {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.root().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for JsonDoc {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Value::deserialize(deserializer).map(|value| JsonDoc::from_value(&value))
    }
}
