//! Reading the object and putting it in the buffer.
//!
//! The `get` runs on the Kubernetes runtime (`oxikube_runtime::spawn_kube`, aborted with the
//! task), the render on the background executor; only the final `set_text` and the decorations
//! run on the UI thread. A table row may be metadata only (ADR 0006), so the editor never starts
//! from one: it always asks the cluster. The toggles (`managedFields`, Secret values) render the
//! object already held, again off the UI thread, and are refused while the buffer holds edits,
//! because showing a different rendering would throw them away.

use std::sync::Arc;

use gpui::{AppContext as _, Context, Window};
use oxikube_domain::{OxiError, Resource};
use oxikube_ports::ResourceReader;
use oxikube_ui::editor::{Decoration, DecorationStyle, EditorApi as _};
use oxikube_workspace::ItemEvent;

use super::binding::{OpenMode, ResourceLoad};
use super::render::{RenderedManifest, render_resource};
use crate::view::ManifestEditor;

/// The notice shown when a toggle is refused because the buffer has edits.
const DIRTY_NOTICE: &str = "Undo your edits before changing what the text shows.";

impl ManifestEditor {
    /// Reads the object (a fresh `get`) and shows it. Called when the editor opens and again when
    /// a failed one is opened a second time.
    pub(crate) fn begin_load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        binding.generation += 1;
        let generation = binding.generation;
        binding.notice = None;
        let (Some(reader), Some(_)) = (binding.reader.clone(), oxikube_runtime::mode(cx)) else {
            binding.load = ResourceLoad::Failed("The cluster is not connected.".to_owned());
            binding.notice = Some("The cluster is not connected.".into());
            return;
        };
        binding.load = ResourceLoad::Loading;
        let target = binding.target.clone();
        let options = binding.options;
        let fetch = oxikube_runtime::spawn_kube(cx, async move {
            reader
                .get(&target.gvk, target.namespace(), &target.name)
                .await
        });
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let read = fetch
                .await
                .unwrap_or_else(|error| Err(OxiError::internal(error.to_string())));
            let resource = match read {
                Ok(resource) => Arc::new(resource),
                Err(error) => {
                    let _ = this.update(cx, |view, cx| view.load_failed(generation, &error, cx));
                    return;
                }
            };
            let held = resource.clone();
            let rendered = cx
                .background_spawn(async move { render_resource(&held, options) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| match rendered {
                Ok(rendered) => view.show(generation, Some(resource), rendered, window, cx),
                Err(error) => {
                    view.load_failed(generation, &OxiError::internal(error.to_string()), cx);
                }
            });
        }));
    }

    fn load_failed(&mut self, generation: u64, error: &OxiError, cx: &mut Context<Self>) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        if binding.generation != generation {
            return;
        }
        tracing::warn!(target = %binding.target, %error, "reading the object for the editor failed");
        let message = error.message().to_owned();
        binding.notice = Some(format!("Could not read the object: {message}").into());
        binding.load = ResourceLoad::Failed(message);
        cx.notify();
    }

    /// Puts `rendered` in the buffer (the object read, or the held one rendered again): the
    /// buffer then counts as unchanged, `status` is dimmed and the validation runs. A result for
    /// an older request is dropped.
    fn show(
        &mut self,
        generation: u64,
        resource: Option<Arc<Resource>>,
        rendered: RenderedManifest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        if binding.generation != generation {
            return;
        }
        let first = resource.is_some();
        if resource.is_some() {
            binding.resource = resource;
        }
        binding.rendered(&rendered);
        let read_only = binding.mode == OpenMode::View || binding.cluster_read_only;
        binding.notice = (binding.mode == OpenMode::Edit && binding.cluster_read_only).then(|| {
            "This cluster is read-only: the tab is read-only and cannot be applied.".into()
        });
        self.editor.update(cx, |editor, cx| {
            let mut api = editor.api(window, cx);
            api.clear_decorations();
            api.set_text(&rendered.text);
            for range in &rendered.dimmed {
                api.add_decoration(Decoration {
                    range: range.clone(),
                    style: DecorationStyle::Dimmed,
                });
            }
            if first {
                api.set_read_only(read_only);
            }
        });
        self.clean_version = self.editor.read(cx).version();
        self.schedule_validation(std::time::Duration::ZERO, window, cx);
        cx.emit(ItemEvent::UpdateTab);
        cx.notify();
    }

    /// Renders the held object again with the binding's options (a toggle changed), off the UI
    /// thread. Refused while the buffer has edits.
    fn rerender(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_dirty(cx) {
            if let Some(binding) = self.binding.as_mut() {
                binding.notice = Some(DIRTY_NOTICE.into());
            }
            cx.notify();
            return;
        }
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        let Some(resource) = binding.resource.clone() else {
            return;
        };
        binding.generation += 1;
        let generation = binding.generation;
        let options = binding.options;
        self.load_task = Some(cx.spawn_in(window, async move |this, cx| {
            let rendered = cx
                .background_spawn(async move { render_resource(&resource, options) })
                .await;
            let _ = this.update_in(cx, |view, window, cx| {
                if let Ok(rendered) = rendered {
                    view.show(generation, None, rendered, window, cx);
                }
            });
        }));
    }

    /// Shows or hides `metadata.managedFields` (`editor::ToggleManagedFields`). Does nothing
    /// until the object is read, and while the buffer has edits (a notice says so).
    pub fn toggle_managed_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        if binding.load != ResourceLoad::Ready {
            return;
        }
        if self.is_dirty(cx) {
            self.rerender(window, cx);
            return;
        }
        if let Some(binding) = self.binding.as_mut() {
            binding.options.managed_fields = !binding.options.managed_fields;
        }
        self.rerender(window, cx);
    }

    /// Shows the values of a Secret, or masks them again (`editor::ToggleSecretMask`). The values
    /// come from the object held in memory. Does nothing for another kind, until the object is
    /// read, and while the buffer has edits (a notice says so).
    pub fn toggle_secret_mask(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        if binding.load != ResourceLoad::Ready || !binding.is_secret() {
            return;
        }
        if self.is_dirty(cx) {
            self.rerender(window, cx);
            return;
        }
        if let Some(binding) = self.binding.as_mut() {
            binding.options.reveal_secrets = !binding.options.reveal_secrets;
        }
        self.rerender(window, cx);
    }

    /// The editor is opened again for the object it shows: "Edit" on a tab that was opened
    /// read-only makes it editable (unless the cluster is read-only); a tab whose read failed
    /// reads again with `reader`. The buffer's edits are never touched.
    pub fn reopen(
        &mut self,
        mode: OpenMode,
        reader: Option<Arc<dyn ResourceReader>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(binding) = self.binding.as_mut() else {
            return;
        };
        let failed = matches!(binding.load, ResourceLoad::Failed(_));
        if failed {
            binding.reader = reader;
            binding.mode = mode;
            self.begin_load(window, cx);
            return;
        }
        if mode == OpenMode::Edit && binding.mode == OpenMode::View {
            binding.mode = OpenMode::Edit;
            if !binding.cluster_read_only {
                self.editor
                    .update(cx, |editor, cx| editor.api(window, cx).set_read_only(false));
            }
            if binding.cluster_read_only {
                binding.notice = Some(
                    "This cluster is read-only: the tab is read-only and cannot be applied.".into(),
                );
            }
            cx.notify();
        }
    }
}
