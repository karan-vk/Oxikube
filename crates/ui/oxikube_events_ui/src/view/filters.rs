//! The toolbar's filters: the four text fields (reason, kind, namespace, text), the type choice
//! and the faults toggle; compiling them into the subscription's filter on every edit, and
//! loading and saving them per cluster.
//!
//! A field updates on the keystroke (the input owns its text); the compile is a few comparisons
//! and, for a regex, one compile per edit. A good filter goes to the subscription at once, whose
//! next item is a reset: the matching rows are read off the UI thread, so typing never waits for
//! 10 000 events to be filtered. A bad regex is shown under the toolbar and the last good text
//! stays in force; the toggles and the other fields still apply on top of it, so the rows always
//! say what the toolbar shows.

use gpui::{AppContext as _, Context, Entity, Window};
use oxikube_ui::input::{InputEvent, InputState};

use super::{EventsView, Fields};
use crate::filter::{EventsFilters, TypeChoice};
use crate::persist::{FiltersWriter, SavedFilters};

/// Which text field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    /// Reasons.
    Reason,
    /// Kinds of the involved object.
    Kind,
    /// Namespaces.
    Namespace,
    /// The text (substring or regex).
    Text,
}

pub(super) fn fields(window: &mut Window, cx: &mut Context<EventsView>) -> Fields {
    let mut field = |placeholder: &'static str| {
        cx.new(|cx| InputState::new(window, cx).placeholder(placeholder))
    };
    Fields {
        reason: field("Reason"),
        kind: field("Kind"),
        namespace: field("Namespace"),
        text: field("Filter: text or regex"),
    }
}

impl Field {
    const ALL: [Field; 4] = [Field::Reason, Field::Kind, Field::Namespace, Field::Text];
}

impl Fields {
    pub(crate) fn get(&self, field: Field) -> &Entity<InputState> {
        match field {
            Field::Reason => &self.reason,
            Field::Kind => &self.kind,
            Field::Namespace => &self.namespace,
            Field::Text => &self.text,
        }
    }
}

impl EventsFilters {
    fn field(&self, field: Field) -> &str {
        match field {
            Field::Reason => &self.reason,
            Field::Kind => &self.kind,
            Field::Namespace => &self.namespace,
            Field::Text => &self.text,
        }
    }

    fn field_mut(&mut self, field: Field) -> &mut String {
        match field {
            Field::Reason => &mut self.reason,
            Field::Kind => &mut self.kind,
            Field::Namespace => &mut self.namespace,
            Field::Text => &mut self.text,
        }
    }
}

impl EventsView {
    pub(super) fn subscribe_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for field in Field::ALL {
            let input = self.fields.get(field).clone();
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |view, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        view.set_field(field, text, cx);
                    }
                    InputEvent::PressEnter { .. } => view.focus_table(window, cx),
                    InputEvent::Focus => view.set_editing(true, cx),
                    InputEvent::Blur => view.set_editing(false, cx),
                },
            );
            self._subscriptions.push(subscription);
        }
    }

    fn set_editing(&mut self, editing: bool, cx: &mut Context<Self>) {
        if self.editing != editing {
            self.editing = editing;
            cx.notify();
        }
    }

    /// A text field's new text (typed, or put there by the view).
    pub fn set_field(&mut self, field: Field, text: String, cx: &mut Context<Self>) {
        let slot = self.filters.field_mut(field);
        if *slot == text {
            return;
        }
        *slot = text;
        self.filters_changed(cx);
    }

    /// The type choice of the toolbar.
    pub fn set_types(&mut self, types: TypeChoice, cx: &mut Context<Self>) {
        if self.filters.types != types {
            self.filters.types = types;
            self.filters_changed(cx);
        }
    }

    /// `events::ToggleFaults`: only the `Warning` events, or every event again.
    pub fn toggle_faults(&mut self, cx: &mut Context<Self>) {
        self.filters.faults_only = !self.filters.faults_only;
        self.filters_changed(cx);
    }

    /// Compiles the filters and sends a new filter to the subscription (its rows are read
    /// again); the filters are saved. A bad regex is shown, and the rest of the filters apply
    /// with the last good text.
    fn filters_changed(&mut self, cx: &mut Context<Self>) {
        let filter = match self.filters.compile() {
            Ok(filter) => {
                self.filter_error = None;
                self.good_text.clone_from(&self.filters.text);
                Some(filter)
            }
            Err(error) => {
                self.filter_error = Some(error.message.into());
                let good = EventsFilters {
                    text: self.good_text.clone(),
                    ..self.filters.clone()
                };
                good.compile().ok()
            }
        };
        if let Some(filter) = filter
            && filter != self.applied
        {
            self.applied = filter.clone();
            self.refiltered = true;
            self.subscription.borrow().set_filter(filter);
        }
        if self.loaded
            && let Some(writer) = &self.writer
        {
            writer.save(self.filters.clone());
        }
        cx.notify();
    }

    /// Puts `filters` in the toolbar and applies them (the saved ones, or a test).
    pub fn set_filters(
        &mut self,
        filters: EventsFilters,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for field in Field::ALL {
            let text = filters.field(field).to_owned();
            self.fields
                .get(field)
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.filters = filters;
        self.filters_changed(cx);
    }

    /// `events::Open` with filters (`:ev kube-system /BackOff`): the namespace and the text
    /// replace what the toolbar had; the rest stays.
    pub fn apply_open(
        &mut self,
        namespace: Option<String>,
        filter: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if namespace.is_none() && filter.is_none() {
            return;
        }
        let mut filters = self.filters.clone();
        filters.namespace = namespace.unwrap_or_default();
        filters.text = filter.unwrap_or_default();
        // Explicit filters win over the saved ones still on their way.
        self.loaded = true;
        self.set_filters(filters, window, cx);
    }

    /// Reads the cluster's saved filters off the UI thread and applies them when they arrive
    /// (unless the user changed a filter meanwhile); saving starts then.
    pub(super) fn load_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let saved = match SavedFilters::new(self.deps.state.clone(), &self.cluster) {
            Ok(saved) => saved,
            Err(error) => {
                tracing::warn!(%error, "the events filters cannot be saved for this cluster");
                self.loaded = true;
                return;
            }
        };
        self.writer = Some(FiltersWriter::spawn(saved.clone(), cx));
        let load = cx.background_spawn(async move { saved.load().await });
        let task = cx.spawn_in(window, async move |this, cx| {
            let loaded = load.await;
            this.update_in(cx, |view, window, cx| view.saved_loaded(loaded, window, cx))
                .ok();
        });
        self.tasks.push(task);
    }

    fn saved_loaded(
        &mut self,
        loaded: oxikube_domain::OxiResult<Option<EventsFilters>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loaded {
            return;
        }
        let untouched = !self.filters.is_active();
        self.loaded = true;
        match loaded {
            Ok(Some(saved)) if untouched => self.set_filters(saved, window, cx),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "reading the saved events filters failed"),
        }
    }
}
