//! Picking a source with the platform file picker, and opening a row's document in the editor.

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{Context, PathPromptOptions, Window};
use oxikube_runtime::spawn_kube;

use super::document::EditorDocument;
use super::source::{ApplySource, SourceKind};
use super::view::ApplyView;

impl ApplyView {
    /// Opens the platform file picker for `kind`; the chosen path is loaded.
    pub fn pick(&mut self, kind: SourceKind, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: kind == SourceKind::Manifests,
            directories: true,
            multiple: false,
            prompt: Some(
                match kind {
                    SourceKind::Manifests => "Apply",
                    SourceKind::Kustomize => "Build with kustomize",
                }
                .into(),
            ),
        });
        // Replacing the slot drops an earlier picker that is still waiting; the task never clears
        // the slot itself.
        self.picker = Some(cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                this.update(cx, |this, cx| {
                    let source = match kind {
                        SourceKind::Manifests => ApplySource::manifests(path),
                        SourceKind::Kustomize => ApplySource::kustomize(path),
                    };
                    this.load(source, cx);
                })
                .ok();
            }
        }));
    }

    /// Opens the selected row's document in the editor, at its source line.
    pub fn open_selected_in_editor(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.model.selected().and_then(|i| self.model.rows().get(i)) else {
            return;
        };
        let (line, source) = (row.line, row.source.clone());
        let cluster = self.deps.cluster.clone();
        let editor = self.deps.editor.clone();
        if source.starts_with("kustomize:") {
            // Built output has no file: the object is shown as YAML from line 1.
            let Some(item) = self
                .plan
                .as_ref()
                .and_then(|plan| plan.plan.items.get(row.index))
            else {
                return;
            };
            let text = serde_saphyr::to_string(&item.doc.json).unwrap_or_default();
            let title = format!("{}-{}.yaml", row.kind.to_lowercase(), row.name);
            editor(
                EditorDocument {
                    cluster,
                    title,
                    text: Arc::from(text),
                    line: 1,
                },
                cx,
            );
            return;
        }
        let path = PathBuf::from(&source);
        let title = path.file_name().map_or_else(
            || source.clone(),
            |name| name.to_string_lossy().into_owned(),
        );
        let read = self.deps.backend.read_text(path);
        self.reader = Some(
            cx.spawn(async move |this, cx| match spawn_kube(cx, read).await {
                Ok(Ok(text)) => {
                    this.update(cx, |_, cx| {
                        editor(
                            EditorDocument {
                                cluster,
                                title,
                                text: Arc::from(text),
                                line,
                            },
                            cx,
                        );
                    })
                    .ok();
                }
                Ok(Err(error)) => {
                    this.update(cx, |this, cx| {
                        this.problem = Some(format!("Could not open {title}: {error}"));
                        cx.notify();
                    })
                    .ok();
                }
                Err(_) => {}
            }),
        );
    }
}
