//! Bounded passive JSON observation. Only a validated model leaves this module;
//! scratch bytes never enter logs/history and are cleared as soon as possible.
use crate::history::Model;
use hyper::{HeaderMap, header};
use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use std::{fmt, sync::OnceLock};

pub(super) const MAX_MODEL_BYTES: usize = 64 * 1024;

/// Upload and terminal response can race. A terminal recorder seals pending
/// metadata as unknown, so neither a late upload nor Drop can revise a record.
#[derive(Default)]
pub(super) struct RequestMetadata {
    model: OnceLock<Option<Model>>,
}
impl RequestMetadata {
    pub(super) fn publish(&self, model: Option<Model>) {
        let _ = self.model.set(model);
    }
    pub(super) fn finish(&self) -> Option<Model> {
        self.model.get_or_init(|| None).clone()
    }
}

pub(super) struct ModelObserver {
    enabled: bool,
    seen: usize,
    buffer: Vec<u8>,
    started: bool,
    depth: usize,
    string: bool,
    escape: bool,
    parsed: bool,
    model: Option<Model>,
}
impl ModelObserver {
    pub(super) fn new(headers: &HeaderMap) -> Self {
        let mut types = headers.get_all(header::CONTENT_TYPE).iter();
        let media = types
            .next()
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        let json = media == "application/json"
            || media.strip_prefix("application/").is_some_and(|s| {
                s.len() > 5
                    && s.ends_with("+json")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            });
        let mut encodings = headers.get_all(header::CONTENT_ENCODING).iter();
        let identity = encodings.next().is_none_or(|v| {
            v.to_str()
                .is_ok_and(|s| s.trim().eq_ignore_ascii_case("identity"))
        });
        Self {
            enabled: json && types.next().is_none() && identity && encodings.next().is_none(),
            seen: 0,
            buffer: Vec::new(),
            started: false,
            depth: 0,
            string: false,
            escape: false,
            parsed: false,
            model: None,
        }
    }

    pub(super) fn feed(&mut self, bytes: &[u8]) {
        if !self.enabled {
            return;
        }
        if bytes.len() > MAX_MODEL_BYTES - self.seen {
            self.disable();
            return;
        }
        self.seen += bytes.len();
        for &byte in bytes {
            if self.parsed {
                if !whitespace(byte) {
                    self.disable();
                    return;
                }
                continue;
            }
            // Geometric Vec growth is bounded by the 64 KiB observation budget.
            self.buffer.push(byte);
            if !self.started {
                if whitespace(byte) {
                    continue;
                }
                if byte != b'{' {
                    self.disable();
                    return;
                }
                self.started = true;
                self.depth = 1;
                continue;
            }
            if self.string {
                if self.escape {
                    self.escape = false;
                } else if byte == b'\\' {
                    self.escape = true;
                } else if byte == b'"' {
                    self.string = false;
                }
            } else {
                match byte {
                    b'"' => self.string = true,
                    b'{' | b'[' => self.depth += 1,
                    b'}' | b']' => {
                        self.depth -= 1;
                        if self.depth == 0 {
                            // This scanner finds only a candidate boundary; serde
                            // validates the entire value, UTF-8, escapes and syntax.
                            // Check raw UTF-8 as well as JSON string escapes;
                            // unobserved fields must still be valid JSON.
                            self.model = std::str::from_utf8(&self.buffer)
                                .ok()
                                .and_then(|text| serde_json::from_str::<TopLevelModel>(text).ok())
                                .and_then(|value| value.0);
                            self.parsed = true;
                            self.clear_buffer();
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Strict policy: a complete JSON object AND real upload EOS are required.
    /// An early model token is never evidence that the upload completed. Large,
    /// cancelled, erroneous or unsupported bodies remain unknown, not estimated.
    pub(super) fn finish(mut self, complete: bool) -> Option<Model> {
        if complete && self.enabled && self.parsed {
            self.model.take()
        } else {
            None
        }
    }
    fn clear_buffer(&mut self) {
        self.buffer.fill(0);
        self.buffer = Vec::new();
    }
    fn disable(&mut self) {
        self.enabled = false;
        self.model = None;
        self.clear_buffer();
    }
}
impl Drop for ModelObserver {
    fn drop(&mut self) {
        self.clear_buffer();
    }
}
fn whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\r' | b'\n' | b'\t')
}

// Selective deserialization: arbitrary prompt/response values are validated and
// skipped, not materialized as serde_json::Value or retained as metadata.
struct TopLevelModel(Option<Model>);
impl<'de> Deserialize<'de> for TopLevelModel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Object;
        impl<'de> Visitor<'de> for Object {
            type Value = TopLevelModel;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object with at most one string model")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut seen = false;
                let mut model = None;
                while let Some(key) = map.next_key::<ModelKey>()? {
                    if key.0 {
                        if seen {
                            return Err(de::Error::custom("duplicate model"));
                        }
                        seen = true;
                        model = map.next_value::<ModelString>()?.0;
                    } else {
                        map.next_value::<SkippedValue>()?;
                    }
                }
                Ok(TopLevelModel(model))
            }
        }
        deserializer.deserialize_map(Object)
    }
}
// IgnoredAny intentionally skips Unicode escape validation. Walk ignored
// structures with a no-retention visitor instead, so malformed prompt strings
// (including lone surrogates) cannot make the whole document appear valid.
struct SkippedValue;
impl<'de> Deserialize<'de> for SkippedValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Skip;
        impl<'de> Visitor<'de> for Skip {
            type Value = SkippedValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a valid JSON value")
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
                Ok(SkippedValue)
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut sequence: A,
            ) -> Result<Self::Value, A::Error> {
                while sequence.next_element::<SkippedValue>()?.is_some() {}
                Ok(SkippedValue)
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                while map.next_key::<ModelKey>()?.is_some() {
                    map.next_value::<SkippedValue>()?;
                }
                Ok(SkippedValue)
            }
        }
        deserializer.deserialize_any(Skip)
    }
}

struct ModelKey(bool);
impl<'de> Deserialize<'de> for ModelKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Key;
        impl Visitor<'_> for Key {
            type Value = ModelKey;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object key")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ModelKey(value == "model"))
            }
        }
        deserializer.deserialize_str(Key)
    }
}
struct ModelString(Option<Model>);
impl<'de> Deserialize<'de> for ModelString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct StringValue;
        impl Visitor<'_> for StringValue {
            type Value = ModelString;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a model string")
            }
            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                Ok(ModelString(Model::new(value).ok()))
            }
        }
        deserializer.deserialize_str(StringValue)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        headers
    }
    fn observe(bytes: &[u8], complete: bool) -> Option<Model> {
        let mut observer = ModelObserver::new(&headers());
        for byte in bytes.chunks(1) {
            observer.feed(byte);
            assert!(observer.buffer.len() <= MAX_MODEL_BYTES);
            assert!(observer.buffer.capacity() <= MAX_MODEL_BYTES);
        }
        observer.finish(complete)
    }

    #[test]
    fn fragmented_escaped_keys_values_and_nested_models() {
        let bytes =
            br#" {"prompt":{"model":"nested","text":"}\\\"["},"mo\u0064el":"test\u002dmodel"} "#;
        assert_eq!(observe(bytes, true).unwrap().as_str(), "test-model");
        assert_eq!(observe(bytes, false), None);
        assert_eq!(observe(br#"{"prompt":{"model":"nested"}}"#, true), None);
        assert_eq!(
            observe(
                "{\"prompt\":\"你好🌏\",\"model\":\"utf8-prompt\"}".as_bytes(),
                true
            )
            .unwrap()
            .as_str(),
            "utf8-prompt"
        );
    }
    #[test]
    fn ambiguous_invalid_incomplete_and_non_string_are_unknown() {
        for bytes in [
            br#"{"model":"one","model":"one"}"#.as_slice(),
            br#"{"model":"one","mo\u0064el":"two"}"#,
            br#"{"model":1}"#,
            br#"{"model":null}"#,
            br#"{"model":["one"]}"#,
            br#"{"model":{"model":"one"}}"#,
            br#"{"model":""}"#,
            br#"{"model":"bad space"}"#,
            br#"{"model":"one""#,
            br#"{"model":"one","prompt":"unfinished"#,
            br#"{"model":"one",}"#,
            br#"{"model":"one"}{}"#,
            br#"{"model":"one","prompt":[true,]}"#,
            br#"[ {"model":"one"} ]"#,
            b"{\"model\":\"one\",\"prompt\":\"\xff\"}",
            br#"{"model":"one","prompt":"\uD800"}"#,
            br#"{"model":"one","prompt":{"\uDC00":true}}"#,
            br#"{"model":"one","prompt":"\x00"}"#,
        ] {
            assert_eq!(observe(bytes, true), None, "{bytes:?}");
        }
        assert_eq!(
            observe(
                format!("{{\"model\":\"{}\"}}", "x".repeat(129)).as_bytes(),
                true
            ),
            None
        );
        let nested = format!(
            "{{\"model\":\"one\",\"prompt\":{}null{}}}",
            "[".repeat(200),
            "]".repeat(200)
        );
        assert_eq!(observe(nested.as_bytes(), true), None);
    }
    #[test]
    fn budget_and_buffer_lifetime_are_bounded() {
        let mut observer = ModelObserver::new(&headers());
        observer.feed(br#"{"model":"one"}"#);
        assert!(observer.buffer.is_empty());
        assert_eq!(observer.buffer.capacity(), 0);
        observer.feed(&vec![b' '; MAX_MODEL_BYTES]);
        assert!(!observer.enabled);
        assert_eq!(observer.finish(true), None);
        let mut bytes = br#"{"model":"one"}"#.to_vec();
        bytes.resize(MAX_MODEL_BYTES, b' ');
        assert_eq!(observe(&bytes, true).unwrap().as_str(), "one");
        bytes.push(b' ');
        assert_eq!(observe(&bytes, true), None);
        let mut observer = ModelObserver::new(&headers());
        observer.feed(&vec![b' '; MAX_MODEL_BYTES]);
        assert_eq!(observer.buffer.len(), MAX_MODEL_BYTES);
        observer.feed(b"{");
        assert!(observer.buffer.is_empty());
        assert_eq!(observer.buffer.capacity(), 0);
        assert_eq!(observer.finish(true), None);
    }
    #[test]
    fn original_content_headers_must_be_unambiguous_json_identity() {
        for (name, value) in [
            (header::CONTENT_TYPE, "text/plain"),
            (header::CONTENT_TYPE, "application/json, application/json"),
            (
                header::CONTENT_TYPE,
                "application/json, application/vendor+json",
            ),
            (header::CONTENT_TYPE, "application/vendor with spaces+json"),
            (header::CONTENT_ENCODING, "gzip"),
            (header::CONTENT_ENCODING, "identity, identity"),
            (header::CONTENT_ENCODING, ""),
        ] {
            let mut headers = headers();
            headers.insert(name, value.parse().unwrap());
            assert!(!ModelObserver::new(&headers).enabled);
        }
        for name in [header::CONTENT_TYPE, header::CONTENT_ENCODING] {
            let mut headers = headers();
            if name == header::CONTENT_ENCODING {
                headers.insert(name.clone(), "identity".parse().unwrap());
            }
            headers.append(name, "identity".parse().unwrap());
            assert!(!ModelObserver::new(&headers).enabled);
        }
        let mut headers = headers();
        headers.insert(
            header::CONTENT_TYPE,
            "Application/vnd.test+json; charset=utf-8".parse().unwrap(),
        );
        headers.insert(header::CONTENT_ENCODING, "Identity".parse().unwrap());
        assert!(ModelObserver::new(&headers).enabled);
    }
    #[test]
    fn terminal_metadata_is_sealed_once_even_if_upload_finishes_later() {
        let metadata = RequestMetadata::default();
        assert_eq!(metadata.finish(), None);
        metadata.publish(Some(Model::new("late").unwrap()));
        assert_eq!(metadata.finish(), None);
        let metadata = RequestMetadata::default();
        metadata.publish(Some(Model::new("first").unwrap()));
        metadata.publish(None);
        assert_eq!(metadata.finish().unwrap().as_str(), "first");
    }
}
