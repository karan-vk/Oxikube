//! Decoding a release payload: base64 (twice for Secrets), gzip, JSON.
//!
//! Pure and synchronous; [`super::reader`] runs it on a blocking thread. Every failure is a
//! [`HelmError::Parse`] whose text names the step but never quotes the payload (it can hold
//! credentials). Inflated output is capped at [`MAX_DECODED_BYTES`] so a hostile cluster
//! cannot make the client allocate without bound (a gzip bomb).

use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use flate2::read::GzDecoder;
use oxikube_domain::helm::HelmError;
use serde::de::DeserializeOwned;

use super::storage::Driver;

/// The most bytes a decoded release may inflate to. Helm itself caps a release object at
/// the API server's 1 MiB object limit compressed; real manifests stay far below this.
pub(crate) const MAX_DECODED_BYTES: u64 = 50 * 1024 * 1024;

/// The first two bytes of a gzip stream.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Decodes the `data.release` string of a stored revision into Helm's release JSON, then
/// into `T` (unknown fields ignored).
///
/// For [`Driver::Secret`] `data` is the Secret's data value as the API returns it (base64 of
/// Helm's own base64); for [`Driver::ConfigMap`] it is Helm's base64 directly. Helm always
/// gzips, but the magic bytes decide: an uncompressed record decodes too.
///
/// # Errors
///
/// [`HelmError::Parse`] for bad base64, a truncated or oversized gzip stream, or JSON that
/// does not have `T`'s shape.
pub(crate) fn decode<T: DeserializeOwned>(driver: Driver, data: &str) -> Result<T, HelmError> {
    let bytes = helm_bytes(driver, data)?;
    let json = inflate(&bytes, MAX_DECODED_BYTES)?;
    serde_json::from_slice(&json).map_err(|e| {
        // serde_json's message carries a position and a category, never input text.
        parse(format!("the release record is not valid JSON ({e})"))
    })
}

/// Undoes the base64 layers, giving Helm's (normally gzipped) bytes.
fn helm_bytes(driver: Driver, data: &str) -> Result<Vec<u8>, HelmError> {
    let once = base64(data.trim(), "release data")?;
    match driver {
        Driver::ConfigMap => Ok(once),
        Driver::Secret => {
            let text = std::str::from_utf8(&once)
                .map_err(|_| parse("the Secret's release data is not Helm's base64 text"))?;
            base64(text.trim(), "Helm's release encoding")
        }
    }
}

fn base64(text: &str, what: &str) -> Result<Vec<u8>, HelmError> {
    STANDARD
        .decode(text)
        .map_err(|e| parse(format!("{what} is not valid base64 ({e})")))
}

/// Gunzips `bytes` (when they start with the gzip magic) into at most `cap` bytes.
pub(crate) fn inflate(bytes: &[u8], cap: u64) -> Result<Vec<u8>, HelmError> {
    if !bytes.starts_with(&GZIP_MAGIC) {
        return if bytes.len() as u64 > cap {
            Err(too_large(cap))
        } else {
            Ok(bytes.to_vec())
        };
    }
    let mut out = Vec::with_capacity(bytes.len().saturating_mul(4));
    GzDecoder::new(bytes)
        .take(cap + 1)
        .read_to_end(&mut out)
        .map_err(|e| {
            parse(format!(
                "the release record is not a valid gzip stream ({e})"
            ))
        })?;
    if out.len() as u64 > cap {
        return Err(too_large(cap));
    }
    Ok(out)
}

fn too_large(cap: u64) -> HelmError {
    parse(format!(
        "the release record inflates to more than {} MiB",
        cap / (1024 * 1024)
    ))
}

fn parse(message: impl Into<String>) -> HelmError {
    HelmError::Parse(message.into())
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::{Value, json};

    use super::*;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    fn secret_data(bytes: &[u8]) -> String {
        STANDARD.encode(STANDARD.encode(bytes))
    }

    fn expect_parse(result: Result<Value, HelmError>, needle: &str) {
        match result {
            Err(HelmError::Parse(message)) => assert!(message.contains(needle), "{message}"),
            other => panic!("expected a parse error about {needle:?}, got {other:?}"),
        }
    }

    #[test]
    fn secret_and_configmap_encodings_decode() {
        let record = json!({"name": "web", "version": 3});
        let gz = gzip(record.to_string().as_bytes());
        let secret: Value = decode(Driver::Secret, &secret_data(&gz)).unwrap();
        let configmap: Value = decode(Driver::ConfigMap, &STANDARD.encode(&gz)).unwrap();
        assert_eq!(secret, record);
        assert_eq!(configmap, record);
    }

    #[test]
    fn an_uncompressed_record_is_detected_by_its_missing_magic() {
        let record = json!({"name": "plain"});
        let data = STANDARD.encode(record.to_string());
        assert_eq!(decode::<Value>(Driver::ConfigMap, &data).unwrap(), record);
    }

    #[test]
    fn bad_base64_is_a_parse_error() {
        expect_parse(decode(Driver::ConfigMap, "not base64!!"), "base64");
        // Valid outer base64 around invalid inner base64.
        expect_parse(
            decode(Driver::Secret, &STANDARD.encode("%%%")),
            "Helm's release encoding",
        );
    }

    #[test]
    fn a_truncated_gzip_stream_is_a_parse_error() {
        let gz = gzip(json!({"name": "web"}).to_string().as_bytes());
        let truncated = &gz[..gz.len() - 6];
        expect_parse(
            decode(Driver::ConfigMap, &STANDARD.encode(truncated)),
            "gzip",
        );
    }

    #[test]
    fn truncated_json_is_a_parse_error_without_echoing_the_payload() {
        let gz = gzip(br#"{"name": "web", "config": {"password": "hunter2""#);
        let result = decode::<Value>(Driver::ConfigMap, &STANDARD.encode(gz));
        let Err(HelmError::Parse(message)) = result else {
            panic!("expected a parse error");
        };
        assert!(message.contains("JSON"), "{message}");
        assert!(!message.contains("hunter2"), "{message}");
    }

    #[test]
    fn inflating_past_the_cap_is_refused() {
        let bomb = gzip(&vec![b' '; 64 * 1024]);
        assert!(inflate(&bomb, 1024 * 1024).is_ok());
        match inflate(&bomb, 1024) {
            Err(HelmError::Parse(message)) => assert!(message.contains("inflates")),
            other => panic!("expected the cap to trip, got {other:?}"),
        }
        assert!(
            inflate(&[b'x'; 2048], 1024).is_err(),
            "raw bytes are capped too"
        );
    }
}
