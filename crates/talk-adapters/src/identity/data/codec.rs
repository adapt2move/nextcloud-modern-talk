//! Bounded serialization; typed limits are rechecked on every private-file read.
use super::{DataError, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de, ser::SerializeSeq};
use std::{fmt, marker::PhantomData};
use talk_core::{
    bounded::{Page, Text},
    preferences::navigation::NavigationRoomKey,
};

struct Writer {
    bytes: Vec<u8>,
    limit: usize,
}
impl std::io::Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::ErrorKind::FileTooLarge.into());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode<T: Serialize>(value: &T, limit: usize) -> Result<Vec<u8>> {
    let mut writer = Writer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|_| DataError::Capacity)?;
    Ok(writer.bytes)
}
pub(super) fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8], limit: usize) -> Result<T> {
    if bytes.len() > limit {
        return Err(DataError::Corrupt);
    }
    serde_json::from_slice(bytes).map_err(|_| DataError::Corrupt)
}
pub(super) mod text {
    use super::*;
    pub fn serialize<S: Serializer, const N: usize>(
        value: &Text<N>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> std::result::Result<Text<N>, D::Error> {
        struct Limited<const N: usize>;
        impl<'de, const N: usize> de::Visitor<'de> for Limited<N> {
            type Value = Text<N>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a string of at most {N} bytes")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Self::Value, E> {
                Text::new(value).map_err(|_| E::custom("text limit"))
            }
        }
        deserializer.deserialize_str(Limited::<N>)
    }
}
pub(super) mod optional_text {
    use super::*;
    pub fn serialize<S: Serializer, const N: usize>(
        value: &Option<Text<N>>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match value {
            Some(value) => serializer.serialize_some(value.as_str()),
            None => serializer.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        deserializer: D,
    ) -> std::result::Result<Option<Text<N>>, D::Error> {
        struct Optional<const N: usize>;
        impl<'de, const N: usize> de::Visitor<'de> for Optional<N> {
            type Value = Option<Text<N>>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "null or text of at most {N} bytes")
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(None)
            }
            fn visit_some<D: Deserializer<'de>>(
                self,
                deserializer: D,
            ) -> std::result::Result<Self::Value, D::Error> {
                text::deserialize(deserializer).map(Some)
            }
        }
        deserializer.deserialize_option(Optional::<N>)
    }
}
pub(super) mod page {
    use super::*;
    pub fn serialize<S: Serializer, T: Serialize, const N: usize>(
        value: &Page<T, N>,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(value.as_slice().len()))?;
        for item in value.as_slice() {
            sequence.serialize_element(item)?;
        }
        sequence.end()
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
        deserializer: D,
    ) -> std::result::Result<Page<T, N>, D::Error> {
        struct Limited<T, const N: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const N: usize> de::Visitor<'de> for Limited<T, N> {
            type Value = Page<T, N>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a list of at most {N} entries")
            }
            fn visit_seq<A: de::SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                if sequence.size_hint().is_some_and(|size| size > N) {
                    return Err(de::Error::custom("page limit"));
                }
                let mut result = Page::empty();
                while let Some(item) = sequence.next_element()? {
                    result
                        .try_push(item)
                        .map_err(|_| de::Error::custom("page limit"))?;
                }
                Ok(result)
            }
        }
        deserializer.deserialize_seq(Limited::<T, N>(PhantomData))
    }
}
pub(super) mod room_key {
    use super::*;
    pub fn serialize<S: Serializer>(
        value: &NavigationRoomKey,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(value.as_str())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<NavigationRoomKey, D::Error> {
        let value: Text<256> = text::deserialize(deserializer)?;
        NavigationRoomKey::new(value.as_str()).map_err(|_| de::Error::custom("room key"))
    }
}
