//! The pod detail panel (E12-S03): what a pod's detail shows above the generic Overview.
//!
//! ```text
//! actions    Logs  Evict  Delete  Force delete  Jump to owner  Show node
//! containers the sub-table of E12-S02 (ContainersPanel)
//! sections   OWNER CHAIN   Owned by ReplicaSet web-5d, …owned by Deployment web   (open)
//!            NODE          Node worker-1                                          (open)
//!            ENVIRONMENT   NAME = value, Secret values masked, references resolved
//!            MOUNTS        PROBES        VOLUMES        TOLERATIONS        AFFINITY
//! ```
//!
//! The generic Overview below keeps the metadata (labels, annotations, finalizers), the
//! conditions and the Events. The panel's own data is built once per version of the pod, in the
//! observer of the detail view, never in `render`:
//!
//! * the sections come from the domain's `PodDetail` (`model`);
//! * the owner chain is read once per owner (`owner_chain`, one `get` per level, through
//!   `spawn_kube`);
//! * the environment is read when its section is first opened: one `get` for each ConfigMap and
//!   Secret the containers name (`env_sources`), never a feed. A Secret's value is decoded into a
//!   `SecretValue`, shown masked, and revealed per variable on request; it lives in this entity
//!   only (non-negotiable 5) and goes nowhere else: not into a log, an audit record or a file.
//!
//! | File | Holds |
//! |---|---|
//! | `mod` | [`PodPanel`]: state, the observer, the reads |
//! | `section` | [`SectionId`]: the sections, their titles and which start open |
//! | `env_rows` | the environment rows: a resolved variable, its value or the chip of a missing reference |
//! | `model` | [`Section`]s and [`Row`]s as data, built from the domain view-model |
//! | `render` | drawing: the action bar, the sections, the rows |
//! | `env_line` | drawing one environment variable: name, masked or revealed value, origin |
//! | `actions` | the action bar's buttons and the commands they send |

mod actions;
mod env_line;
mod env_rows;
mod model;
mod render;
mod section;
#[cfg(test)]
pub(crate) mod tests;

use std::collections::HashSet;
use std::sync::Arc;

use gpui::{AnyView, App, AppContext as _, Context, Task, WeakEntity, Window};
use oxikube_domain::ids::ResourceRef;
use oxikube_domain::json::JsonDoc;
use oxikube_domain::view::SourceKind;
use oxikube_runtime::spawn_kube;

pub use env_rows::EnvCell;
pub use model::{EnvKey, EnvLine, Row, Section};
use model::{Loaded, PodModel};
pub use section::SectionId;

use super::containers::ContainersPanel;
use crate::detail::DetailView;

/// The pod panel of one pod detail. See the [module docs](self).
pub struct PodPanel {
    detail: WeakEntity<DetailView>,
    /// The containers sub-table, drawn above the sections.
    containers: AnyView,
    /// The document the model was built from.
    doc: Option<JsonDoc>,
    model: Option<PodModel>,
    /// The sections that are open.
    open: HashSet<SectionId>,
    /// The Secret values the user asked to see.
    revealed: HashSet<EnvKey>,
    loaded: Loaded,
    /// The sections as drawn: rebuilt when the pod, a read or the open set changes.
    sections: Vec<Section>,
    /// The owners the chain was read for (kind and name of each owner reference).
    chain_for: Vec<(String, String)>,
    /// The ConfigMaps and Secrets the environment was read for; `None` until it was read, so a
    /// pod that names none (literal and field-reference variables only) is still read once.
    env_for: Option<Vec<(SourceKind, Arc<str>)>>,
    chain_task: Option<Task<()>>,
    env_task: Option<Task<()>>,
    /// How many times the model was built (tests: once per pod version).
    builds: usize,
}

impl PodPanel {
    /// The panel of `detail` (the pod's [`DetailRenderer`](crate::kinds::DetailRenderer) factory).
    ///
    /// The detail is being drawn when it calls this, so the first model is built right after
    /// (deferred); until then the panel draws its containers only.
    pub fn build(detail: WeakEntity<DetailView>, window: &mut Window, cx: &mut App) -> AnyView {
        let containers = ContainersPanel::build(detail.clone(), window, cx);
        let panel = cx.new(|cx| {
            if let Some(view) = detail.upgrade() {
                cx.observe(&view, |this: &mut Self, _, cx| this.refresh(cx))
                    .detach();
            }
            Self {
                detail,
                containers,
                doc: None,
                model: None,
                open: SectionId::ALL
                    .into_iter()
                    .filter(|id| id.starts_open())
                    .collect(),
                revealed: HashSet::new(),
                loaded: Loaded::default(),
                sections: Vec::new(),
                chain_for: Vec::new(),
                env_for: None,
                chain_task: None,
                env_task: None,
                builds: 0,
            }
        });
        let first = panel.downgrade();
        cx.defer(move |cx| {
            first.update(cx, |panel, cx| panel.refresh(cx)).ok();
        });
        panel.into()
    }

    /// The containers sub-table.
    pub fn containers(&self) -> &AnyView {
        &self.containers
    }

    /// The sections as drawn now.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Whether `id` is open.
    pub fn is_open(&self, id: SectionId) -> bool {
        self.open.contains(&id)
    }

    /// Whether the value of the variable `key` is shown.
    pub fn is_revealed(&self, key: &EnvKey) -> bool {
        self.revealed.contains(key)
    }

    /// How many times the model was built.
    pub fn builds(&self) -> usize {
        self.builds
    }

    fn target(&self, cx: &App) -> Option<ResourceRef> {
        Some(self.detail.upgrade()?.read(cx).target().clone())
    }

    /// Rebuilds the model when the detail shows another version of the pod.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let built = detail.read(cx).resource().and_then(|pod| {
            let same = self.doc.as_ref().is_some_and(|doc| doc.ptr_eq(pod.doc()));
            (!same).then(|| (pod.doc().clone(), PodModel::new(pod)))
        });
        let Some((doc, model)) = built else {
            return;
        };
        self.doc = Some(doc);
        self.model = model;
        self.builds += 1;
        self.read_what_changed(cx);
        self.rebuild(cx);
    }

    /// Starts the reads the new version of the pod needs: the owner chain when its owners
    /// changed, the environment when it is open and names other objects than the last read did.
    fn read_what_changed(&mut self, cx: &mut Context<Self>) {
        let Some(model) = &self.model else {
            return;
        };
        let owners: Vec<(String, String)> = model
            .detail
            .owners
            .iter()
            .map(|o| (o.kind.to_string(), o.name.to_string()))
            .collect();
        let refs_changed = self.env_stale(model);
        if owners != self.chain_for {
            self.chain_for = owners;
            self.loaded.chain = None;
            self.loaded.chain_error = None;
            self.load_chain(cx);
        }
        if self.open.contains(&SectionId::Environment) && refs_changed {
            self.load_env(cx);
        }
    }

    /// Whether the environment was never read, or was read for other objects than `model` names.
    fn env_stale(&self, model: &PodModel) -> bool {
        self.env_for.as_ref() != Some(&model.detail.source_refs())
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        if let (Some(model), Some(target)) = (&self.model, self.target(cx)) {
            let open = &self.open;
            self.sections = model.sections(&target, &|id| open.contains(&id), &self.loaded);
        } else {
            self.sections.clear();
        }
        cx.notify();
    }

    /// Opens or closes a section. Opening the environment for the first time reads the objects it
    /// names.
    pub fn toggle(&mut self, id: SectionId, cx: &mut Context<Self>) {
        if !self.open.remove(&id) {
            self.open.insert(id);
            if id == SectionId::Environment
                && self.model.as_ref().is_some_and(|m| self.env_stale(m))
            {
                self.load_env(cx);
            }
        }
        self.rebuild(cx);
    }

    /// Shows or hides the Secret value of the variable `key`.
    pub fn toggle_reveal(&mut self, key: EnvKey, cx: &mut Context<Self>) {
        if !self.revealed.remove(&key) {
            self.revealed.insert(key);
        }
        cx.notify();
    }

    fn load_chain(&mut self, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.upgrade() else {
            return;
        };
        let (service, pod) = {
            let detail = detail.read(cx);
            (detail.workload_service(), detail.target().clone())
        };
        let read = spawn_kube(cx, async move { service.owner_chain(&pod).await });
        // A newer read replaces, and so cancels, this one; the task never clears itself.
        self.chain_task = Some(cx.spawn(async move |this, cx| {
            let answer = match read.await {
                Ok(answer) => answer,
                Err(error) => Err(error.into()),
            };
            this.update(cx, |panel, cx| {
                match answer {
                    Ok(chain) => {
                        panel.loaded.chain = Some(chain);
                        panel.loaded.chain_error = None;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "reading the owner chain failed");
                        panel.loaded.chain_error = Some(error.message().to_owned());
                    }
                }
                panel.rebuild(cx);
            })
            .ok();
        }));
    }

    fn load_env(&mut self, cx: &mut Context<Self>) {
        let (Some(detail), Some(model)) = (self.detail.upgrade(), &self.model) else {
            return;
        };
        let refs = model.detail.source_refs();
        let pod = model.pod.clone();
        let (service, cluster) = {
            let detail = detail.read(cx);
            (detail.workload_service(), detail.target().cluster.clone())
        };
        self.env_for = Some(refs);
        self.loaded.env_loading = true;
        self.loaded.env_error = None;
        let read = spawn_kube(cx, async move { service.env_sources(&cluster, &pod).await });
        self.env_task = Some(cx.spawn(async move |this, cx| {
            let answer = match read.await {
                Ok(answer) => answer,
                Err(error) => Err(error.into()),
            };
            this.update(cx, |panel, cx| {
                panel.loaded.env_loading = false;
                match answer {
                    Ok(sources) => {
                        panel.loaded.env = Some(sources);
                        panel.loaded.env_error = None;
                    }
                    Err(error) => {
                        tracing::debug!(%error, "reading the environment's sources failed");
                        // Forget what the read was for, so opening the section tries again.
                        panel.env_for = None;
                        panel.loaded.env = None;
                        panel.loaded.env_error = Some(error.message().to_owned());
                    }
                }
                panel.rebuild(cx);
            })
            .ok();
        }));
        self.rebuild(cx);
    }
}
