//! The full object and the owners' scopes: the two reads that follow the store's object.
//!
//! A feed that delivers metadata only (Secrets, ConfigMaps) or Table rows (custom resources)
//! leaves `spec` and `status` unknown, so the view reads the object once (`ResourceReader::get`
//! on the Tokio bridge through `spawn_kube`, secret values removed inside that task) and again
//! whenever the store reports a new version. Owner links need to know whether the owner's kind is
//! namespaced; discovery answers off-thread, once per kind.

use std::sync::Arc;

use gpui::Context;
use oxikube_domain::ids::{Gvk, Scope};
use oxikube_domain::{ErrorKind, OxiError, OxiResult, Resource};
use oxikube_runtime::spawn_kube;

use super::model::mask_secret;
use super::state::FullState;
use super::view::DetailView;

impl DetailView {
    /// Reads the complete object, keeping what was shown while it is read.
    pub(super) fn fetch_full(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.resources())
        else {
            return;
        };
        if !matches!(self.full, FullState::Loaded(_)) {
            self.full = FullState::Loading;
        }
        let target = self.target.clone();
        let read = spawn_kube(cx, async move {
            let mut resource: Resource = reader
                .get(&target.gvk, target.namespace(), &target.name)
                .await?;
            // Secret values never leave this task.
            mask_secret(&mut resource);
            OxiResult::Ok(resource)
        });
        self.full_task = Some(cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |view, cx| view.full_read(result, cx)).ok();
        }));
    }

    fn full_read(&mut self, result: OxiResult<Resource>, cx: &mut Context<Self>) {
        match result {
            Ok(resource) => self.full = FullState::Loaded(Arc::new(resource)),
            // Gone since the feed's last word: the feed says so in its own time.
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.full = FullState::Idle;
            }
            Err(error) => {
                tracing::warn!(%error, target = %self.target, "reading the full object failed");
                self.full = FullState::Failed(error.message().to_owned());
            }
        }
        self.rebuild(cx);
    }

    /// Asks discovery whether each owner's kind is namespaced (once per kind), so the owner
    /// links can open the right object.
    pub(super) fn resolve_owners(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let cluster_scoped = self.target.namespace.is_none();
        let mut missing: Vec<Gvk> = Vec::new();
        for owner in &model.owners {
            if self.owner_scopes.contains_key(&owner.gvk) || missing.contains(&owner.gvk) {
                continue;
            }
            missing.push(owner.gvk.clone());
        }
        if missing.is_empty() {
            return;
        }
        if cluster_scoped {
            // The owner of a cluster-scoped object is cluster-scoped.
            for gvk in missing {
                self.owner_scopes.insert(gvk, Some(Scope::Cluster));
            }
            return;
        }
        let Some(discovery) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.discovery())
        else {
            return;
        };
        let resolve = spawn_kube(cx, async move {
            let mut answers = Vec::with_capacity(missing.len());
            for gvk in missing {
                // Only an answer is kept: `None` is "not served". A failed lookup is left
                // unanswered, so the next rebuild asks again.
                if let Ok(kind) = discovery.resolve(&gvk).await {
                    answers.push((gvk, kind.map(|kind| kind.scope())));
                }
            }
            answers
        });
        self.owners_task = Some(cx.spawn(async move |this, cx| {
            if let Ok(answers) = resolve.await {
                this.update(cx, |view, cx| {
                    view.owner_scopes.extend(answers);
                    cx.notify();
                })
                .ok();
            }
        }));
    }
}
