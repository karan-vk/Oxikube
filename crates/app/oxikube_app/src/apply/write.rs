//! [`ObjectWrite`]: how one object reaches the server. The only code that writes a manifest,
//! shared by the `resource::Apply` handler (one object) and the `resource::ApplyPlan` handler
//! (each object of a plan), both through the guard's [`Mutation`](crate::guard::Mutation) writer.
//!
//! ```text
//! rehearse:            SSA dry run ─unsupported─> replace / create dry run
//! send (First):        SSA dry run ─ok─> SSA
//!                           └─unsupported─> replace / create dry run ─> replace / create
//! send (Unavailable):  SSA ─unsupported─> replace / create
//! ```
//!
//! Server-side apply runs under the field manager with `force` only when the user chose it. A
//! server that cannot server-side apply the kind answers 415 (some aggregated APIs, old CRD
//! setups), which the adapter reports as `Unsupported`: the object is then replaced (PUT)
//! carrying the reviewed `resourceVersion`, so a change made since fails with a conflict instead
//! of being overwritten, or created when it does not exist. A create the user reviewed stays a
//! create: an object made since fails with `AlreadyExists` instead of being replaced unseen.

use oxikube_domain::ids::ResourceRef;
use oxikube_domain::{ErrorKind, OxiResult, Resource};
use oxikube_ports::{Patch, ResourceReader, ResourceWriter, WriteOptions};
use serde_json::Value;

use super::manifest::with_resource_version;
use super::types::ApplyMethod;

/// Whether the server rehearses a write before it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Preview {
    /// A server dry run first (admission, validation, ownership), then the write.
    First,
    /// The server cannot dry-run the kind (the rehearsal said so): the write is sent without one.
    Unavailable,
}

/// One object to write, as a command describes it.
pub(super) struct ObjectWrite {
    /// The object.
    pub(super) target: ResourceRef,
    /// The request body (no `resourceVersion`, no `managedFields`).
    pub(super) body: Value,
    /// The server-side apply field manager.
    pub(super) manager: String,
    /// Take over other managers' fields.
    pub(super) force: bool,
    /// The live object's version the change was reviewed against; the replace fallback carries it.
    pub(super) resource_version: Option<String>,
    /// The change was reviewed against no live object: the fallback creates.
    pub(super) reviewed_absent: bool,
}

impl ObjectWrite {
    fn namespace(&self) -> Option<&str> {
        self.target.namespace()
    }

    fn options(&self, dry_run: bool) -> WriteOptions {
        WriteOptions {
            dry_run,
            field_manager: Some(self.manager.clone()),
        }
    }

    /// The live object, `None` when it does not exist.
    pub(super) async fn live(&self, reader: &dyn ResourceReader) -> OxiResult<Option<Resource>> {
        let target = &self.target;
        reader
            .get_opt(&target.gvk, self.namespace(), &target.name)
            .await
    }

    /// The server's dry run of the write, next to `live` (the object read just before): the
    /// server-side apply, or the fallback's where the server cannot server-side apply the kind.
    pub(super) async fn rehearse(
        &self,
        writer: &dyn ResourceWriter,
        live: Option<&Resource>,
    ) -> OxiResult<(ApplyMethod, Resource)> {
        match self.server_side(writer, true).await {
            Ok(object) => Ok((ApplyMethod::ServerSide, object)),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                let version = self
                    .resource_version
                    .clone()
                    .or_else(|| version_of(live))
                    .filter(|_| live.is_some());
                self.fallback(writer, version.as_deref(), true).await
            }
            Err(error) => Err(error),
        }
    }

    /// The write itself. With [`Preview::First`] the server rehearses it first (admission,
    /// validation, ownership); with [`Preview::Unavailable`] it is sent straight away, the dry run
    /// being one the server cannot do.
    pub(super) async fn send(
        &self,
        writer: &dyn ResourceWriter,
        reader: &dyn ResourceReader,
        preview: Preview,
    ) -> OxiResult<(ApplyMethod, Resource)> {
        // `None`: the dry run passed and the apply is still to send; `Some`: already sent.
        let server_side = match preview {
            Preview::First => self.server_side(writer, true).await.map(|_| None),
            Preview::Unavailable => self.server_side(writer, false).await.map(Some),
        };
        match server_side {
            Ok(Some(object)) => Ok((ApplyMethod::ServerSide, object)),
            Ok(None) => Ok((
                ApplyMethod::ServerSide,
                self.server_side(writer, false).await?,
            )),
            Err(error) if error.kind() == ErrorKind::Unsupported => {
                // The reviewed version, else none for a reviewed create (an object made since then
                // fails with `AlreadyExists`, never replaced unseen), else the live one for a
                // caller that reviewed nothing.
                let version = match (self.resource_version.clone(), self.reviewed_absent) {
                    (Some(version), _) => Some(version),
                    (None, true) => None,
                    (None, false) => version_of(self.live(reader).await?.as_ref()),
                };
                if preview == Preview::First {
                    self.fallback(writer, version.as_deref(), true).await?;
                }
                self.fallback(writer, version.as_deref(), false).await
            }
            Err(error) => Err(error),
        }
    }

    async fn server_side(&self, writer: &dyn ResourceWriter, dry_run: bool) -> OxiResult<Resource> {
        let patch = Patch::apply(self.body.clone(), self.manager.as_str(), self.force);
        let target = &self.target;
        writer
            .patch(
                &target.gvk,
                self.namespace(),
                &target.name,
                &patch,
                &self.options(dry_run),
            )
            .await
    }

    /// The fallback: a replace carrying `resource_version`, or a create without one.
    async fn fallback(
        &self,
        writer: &dyn ResourceWriter,
        resource_version: Option<&str>,
        dry_run: bool,
    ) -> OxiResult<(ApplyMethod, Resource)> {
        let options = self.options(dry_run);
        let target = &self.target;
        match resource_version {
            Some(version) => {
                let body = with_resource_version(self.body.clone(), version);
                let object = writer
                    .replace(&target.gvk, self.namespace(), &target.name, &body, &options)
                    .await?;
                Ok((ApplyMethod::Replace, object))
            }
            None => {
                let object = writer
                    .create(&target.gvk, self.namespace(), &self.body, &options)
                    .await?;
                Ok((ApplyMethod::Create, object))
            }
        }
    }
}

/// `metadata.resourceVersion` of `live`.
pub(super) fn version_of(live: Option<&Resource>) -> Option<String> {
    live.and_then(|live| live.meta.resource_version.as_deref())
        .map(str::to_owned)
}
