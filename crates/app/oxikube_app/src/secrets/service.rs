//! [`SecretService`]: decoding, encoding and creating Secrets, and the one place that reads a
//! Secret's values for a user who asked to see them.

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::secret::{self, DecodedSecret, SecretError};
use oxikube_domain::{OxiError, OxiResult, Resource};
use oxikube_ports::ResourceReader;
use serde_json::Value;

use super::create::{NewSecret, Problem};
use super::summary::SecretSummary;

/// Decodes Secrets for reveal and edit, encodes manifests back, summarises Secrets without their
/// values and builds new ones. Stateless and cheap to clone: it holds no value of its own.
///
/// Decoded values are plain in-memory types (`SecretText`: `Debug` and `Display` print `***`);
/// nothing here writes one to disk, a log or an audit record. The only reads of a Secret's data
/// are [`read_decoded`](Self::read_decoded) (the user revealed it) and the editor's open of a
/// Secret (`resource::Edit` / `secret::EditDecoded`); both are `get`s on the Kubernetes runtime.
#[derive(Debug, Clone, Copy, Default)]
pub struct SecretService;

impl SecretService {
    /// A service.
    pub fn new() -> Self {
        Self
    }

    /// `resource` with its values decoded.
    ///
    /// # Errors
    ///
    /// [`SecretError::NotASecret`] for any other kind.
    pub fn decode(&self, resource: &Resource) -> Result<DecodedSecret, SecretError> {
        DecodedSecret::from_resource(resource)
    }

    /// What the detail shows of `resource` by default: key names and sizes and the type's public
    /// facts (registries and usernames, the certificate), no value.
    ///
    /// # Errors
    ///
    /// [`SecretError::NotASecret`] for any other kind.
    pub fn summarize(&self, resource: &Resource) -> Result<SecretSummary, SecretError> {
        SecretSummary::of(resource)
    }

    /// Reads the Secret `target` (one `get`) and decodes it, for a user who asked to reveal it.
    /// Logs that the values were read (the object, never a value).
    ///
    /// # Errors
    ///
    /// The read's error (`NotFound`, `Forbidden`, ...), or a validation error when `target` is
    /// not a Secret.
    pub async fn read_decoded(
        &self,
        reader: &dyn ResourceReader,
        target: &ResourceRef,
    ) -> OxiResult<DecodedSecret> {
        let resource = reader
            .get(&target.gvk, target.namespace(), &target.name)
            .await?;
        let decoded = self
            .decode(&resource)
            .map_err(|error| OxiError::validation(error.to_string()))?;
        tracing::info!(
            cluster = %target.cluster,
            namespace = target.namespace().unwrap_or_default(),
            name = %target.name,
            keys = decoded.entries.len(),
            "secret values read for display"
        );
        Ok(decoded)
    }

    /// The object for a new Secret, `data` base64.
    ///
    /// # Errors
    ///
    /// The [`Problem`]s of [`NewSecret::validate`].
    pub fn manifest(&self, new: &NewSecret) -> Result<Value, Vec<Problem>> {
        new.manifest()
    }

    /// Writes the manifest `text` of a decoded-Secret editor as the text to apply: every Secret
    /// in it has its `data` values base64-encoded (a binary placeholder takes the value
    /// `original` had for its key). Other documents pass through. Comments and formatting are
    /// not kept: this text goes to the server only.
    ///
    /// # Errors
    ///
    /// A validation error for text that is not YAML or JSON, or a Secret value that is not text.
    /// The message names keys, never values.
    pub fn encode_manifest(&self, text: &str, original: Option<&Resource>) -> OxiResult<String> {
        // Read as kubectl reads it (YAML 1.1), as the apply does.
        let options = serde_saphyr::options! { legacy_octal_numbers: true };
        // The parser's message quotes the text, which holds Secret values: it is dropped.
        let mut documents: Vec<Value> = serde_saphyr::from_multiple_with_options(text, options)
            .map_err(|_| OxiError::validation("the manifest is not valid YAML or JSON"))?;
        let original = original.map(Resource::to_value);
        let mut changed = false;
        for document in &mut documents {
            if secret::is_secret(document) {
                secret::from_decoded(document, original.as_ref())
                    .map_err(|error| OxiError::validation(error.to_string()))?;
                changed = true;
            }
        }
        if !changed {
            return Ok(text.to_owned());
        }
        let mut out = String::new();
        for (index, document) in documents.iter().enumerate() {
            if index > 0 {
                out.push_str("---\n");
            }
            let yaml = serde_saphyr::to_string(document)
                .map_err(|_| OxiError::internal("the manifest cannot be written as YAML"))?;
            out.push_str(&yaml);
            if !yaml.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use oxikube_domain::ids::{ClusterId, ContextName, Gvk};
    use oxikube_testkit::FakeResourcePort;
    use serde_json::json;

    use super::*;

    fn secret_resource() -> Resource {
        Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "db", "namespace": "ns"},
            "type": "Opaque",
            "data": {"password": "aHVudGVyMg==", "blob": "//4="}
        }))
        .unwrap()
    }

    /// What an editor opened for "edit decoded" shows.
    fn decoded_object(resource: &Resource) -> Value {
        let mut object = resource.to_value();
        secret::to_decoded(&mut object);
        object
    }

    fn target() -> ResourceRef {
        ResourceRef::namespaced(
            ClusterId::new("~/.kube/config", &ContextName::new("kind")),
            Gvk::new("", "v1", "Secret"),
            "ns",
            "db",
        )
    }

    #[test]
    fn decode_and_encode_round_trip() {
        let service = SecretService::new();
        let resource = secret_resource();
        let decoded = service.decode(&resource).unwrap();
        assert_eq!(decoded.text("password"), Some("hunter2"));
        assert_eq!(decoded.encode(), resource.to_value()["data"]);
    }

    #[test]
    fn the_decoded_object_and_the_encoded_manifest_are_inverse() {
        let service = SecretService::new();
        let resource = secret_resource();
        let object = decoded_object(&resource);
        assert_eq!(object["data"]["password"], "hunter2");
        assert_eq!(object["data"]["blob"], "(binary, 2 bytes)");
        let buffer = serde_saphyr::to_string(&object).unwrap();
        let sent = service.encode_manifest(&buffer, Some(&resource)).unwrap();
        let sent: Value = serde_saphyr::from_str(&sent).unwrap();
        assert_eq!(sent["data"], resource.to_value()["data"]);
    }

    #[test]
    fn values_that_look_like_other_yaml_types_survive_the_round_trip() {
        // The YAML writer quotes what a reader could take for a number, a boolean or null, and the
        // encoder reads it back as text: a password of `true`, `0644` or `1e3` is not changed.
        let values = [
            "true",
            "123",
            "null",
            "0644",
            "yes",
            "no",
            "1e3",
            "~",
            "",
            " leading",
            "trailing ",
            "multi\nline\n",
            "no newline at the end\nbut inside",
            "tab\there",
            "emoji \u{1F642}",
            "colon: space",
            "# hash",
            "- dash",
            "{brace}",
            "'quoted'",
            "\"double\"",
            "a\\b",
            "2001-12-14",
            "0x1F",
            "+1",
            "1_000",
            ".5",
            "-.inf",
            "Y",
            "N",
        ];
        let data: serde_json::Map<String, Value> = values
            .iter()
            .enumerate()
            .map(|(index, value)| {
                (
                    format!("key{index}"),
                    Value::String(base64_of(value.as_bytes())),
                )
            })
            .collect();
        let resource = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "Secret",
            "metadata": {"name": "tricky", "namespace": "ns"},
            "data": data
        }))
        .unwrap();
        let service = SecretService::new();
        let buffer = serde_saphyr::to_string(&decoded_object(&resource)).unwrap();
        let sent = service.encode_manifest(&buffer, Some(&resource)).unwrap();
        let sent: Value = serde_saphyr::from_str(&sent).unwrap();
        for (index, value) in values.iter().enumerate() {
            let key = format!("key{index}");
            assert_eq!(
                sent["data"][&key],
                resource.to_value()["data"][&key],
                "{value:?} changed on the way through:\n{buffer}"
            );
        }
    }

    fn base64_of(bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }

    #[test]
    fn editing_the_buffer_changes_only_that_key() {
        let service = SecretService::new();
        let resource = secret_resource();
        let mut object = decoded_object(&resource);
        object["data"]["password"] = json!("new password");
        let buffer = serde_saphyr::to_string(&object).unwrap();
        let sent = service.encode_manifest(&buffer, Some(&resource)).unwrap();
        let sent: Value = serde_saphyr::from_str(&sent).unwrap();
        assert_eq!(sent["data"]["password"], "bmV3IHBhc3N3b3Jk");
        assert_eq!(sent["data"]["blob"], "//4=");
    }

    #[test]
    fn other_documents_pass_through_untouched() {
        let service = SecretService::new();
        let text =
            "# keep me\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: c\ndata:\n  a: b\n";
        assert_eq!(service.encode_manifest(text, None).unwrap(), text);
    }

    #[test]
    fn several_documents_are_each_encoded() {
        let service = SecretService::new();
        let text = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: a\ndata:\n  k: v\n---\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: c\ndata:\n  k: v\n";
        let sent = service.encode_manifest(text, None).unwrap();
        let docs: Vec<Value> = serde_saphyr::from_multiple(&sent).unwrap();
        assert_eq!(docs.len(), 2);
        assert_eq!(docs[0]["data"]["k"], "dg==");
        assert_eq!(docs[1]["data"]["k"], "v");
    }

    #[test]
    fn a_masked_placeholder_is_refused_not_encoded() {
        let service = SecretService::new();
        let text =
            "apiVersion: v1\nkind: Secret\nmetadata:\n  name: a\ndata:\n  password: (hidden)\n";
        let error = service.encode_manifest(text, None).unwrap_err();
        assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
        assert!(error.to_string().contains("password"), "{error}");
        assert!(error.to_string().contains("masked"), "{error}");
    }

    #[test]
    fn errors_never_quote_the_text() {
        let service = SecretService::new();
        let error = service
            .encode_manifest("data: {hunter2: [", None)
            .unwrap_err();
        assert!(!error.to_string().contains("hunter2"), "{error}");
        let number = "apiVersion: v1\nkind: Secret\nmetadata:\n  name: a\ndata:\n  hunter2: 5\n";
        let error = service.encode_manifest(number, None).unwrap_err();
        assert!(
            error.to_string().contains("hunter2"),
            "the key is named: {error}"
        );
    }

    #[test]
    fn read_decoded_reads_once_and_decodes() {
        let service = SecretService::new();
        let port = FakeResourcePort::new().with_objects([secret_resource()]);
        let decoded = futures::executor::block_on(service.read_decoded(&port, &target())).unwrap();
        assert_eq!(decoded.text("password"), Some("hunter2"));
        assert_eq!(port.recorded_calls().len(), 1);
    }

    #[test]
    fn read_decoded_refuses_another_kind() {
        let service = SecretService::new();
        let config = Resource::from_json(json!({
            "apiVersion": "v1", "kind": "ConfigMap",
            "metadata": {"name": "db", "namespace": "ns"}
        }))
        .unwrap();
        let port = FakeResourcePort::new().with_objects([config]);
        let mut wrong = target();
        wrong.gvk = Gvk::new("", "v1", "ConfigMap");
        let error = futures::executor::block_on(service.read_decoded(&port, &wrong)).unwrap_err();
        assert_eq!(error.kind(), oxikube_domain::ErrorKind::Validation);
    }
}
