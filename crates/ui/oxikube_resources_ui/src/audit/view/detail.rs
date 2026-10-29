//! The side panel of the selected record: every field, the other records of its batch, and
//! "Show resource".

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement as _, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use oxikube_app::audit::export::{denial_text, outcome_text};
use oxikube_app::audit::query::MAX_BATCH_RECORDS;
use oxikube_domain::audit::AuditRecord;
use oxikube_domain::command::Command;
use oxikube_runtime::spawn_kube;
use oxikube_ui::button::{Button, ButtonVariants as _};
use oxikube_ui::layout::{h_flex, v_flex};
use oxikube_ui::{ActiveTokens as _, Sizable as _, u};

use super::{AuditView, BatchLoad, BatchState, tagged};
use crate::audit::model::{AuditColumn, cell_text, initiator_text};

/// Batch rows drawn in the panel; the rest is a count.
const MAX_BATCH_ROWS: usize = 100;

impl AuditView {
    /// Selects row `ix` (`None` clears the selection) and opens or closes the panel.
    pub fn select(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if self.table.update_quiet(cx, |d| d.model.select(ix)) {
            self.sync_batch(cx);
            cx.notify();
        }
    }

    /// Moves the selection `delta` rows and scrolls it into view.
    pub fn step(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(ix) = self.table.update_quiet(cx, |d| d.model.step(delta)) {
            self.table.reveal_row(ix, cx);
            self.sync_batch(cx);
            cx.notify();
        }
    }

    /// Reads the batch of the selected record, when it has one and it is not the one shown.
    pub(crate) fn sync_batch(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .model(cx)
            .selected_record()
            .and_then(|record| record.batch_id.clone());
        let Some(id) = wanted else {
            self.batch = None;
            self.batch_task = None;
            return;
        };
        if self.batch.as_ref().is_some_and(|batch| batch.id == id) {
            return;
        }
        self.batch = Some(BatchState {
            id: id.clone(),
            load: BatchLoad::Loading,
        });
        let reader = self.deps.reader.clone();
        let wanted = id.clone();
        let work = spawn_kube(cx, async move { reader.batch(&wanted).await });
        self.batch_task = Some(cx.spawn(async move |this, cx| {
            let result = work.await;
            this.update(cx, |view, cx| {
                // Another batch may have been selected meanwhile.
                let Some(state) = view.batch.as_mut().filter(|state| state.id == id) else {
                    return;
                };
                state.load = match result {
                    Ok(Ok(records)) => BatchLoad::Ready(records),
                    Ok(Err(error)) => BatchLoad::Failed(error.message().to_owned().into()),
                    Err(error) => BatchLoad::Failed(error.to_string().into()),
                };
                cx.notify();
            })
            .ok();
        }));
    }

    /// "Show resource": `resource::Open` for the record's object on the bus. The command says so
    /// when the object no longer exists.
    pub fn show_resource(&mut self, cx: &mut Context<Self>) {
        let Some(record) = self.model(cx).selected_record() else {
            return;
        };
        let target = record.target.clone();
        self.deps
            .dispatcher
            .dispatch(Command::ResourceOpen { target }, cx);
    }

    fn field(
        &self,
        label: &'static str,
        value: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = cx.tokens();
        let value: SharedString = value.into();
        v_flex()
            .gap(u(px(1.)))
            .child(
                div()
                    .text_size(u(tokens.font.small))
                    .text_color(tokens.colors.text_muted)
                    .child(label),
            )
            .child(div().text_size(u(tokens.font.small)).child(value))
            .into_any_element()
    }

    fn batch_section(&self, record: &AuditRecord, cx: &mut Context<Self>) -> Option<AnyElement> {
        let state = self.batch.as_ref()?;
        let tokens = cx.tokens();
        let body: AnyElement = match &state.load {
            BatchLoad::Loading => div().child("Loading…").into_any_element(),
            BatchLoad::Failed(message) => div()
                .text_color(tokens.colors.error)
                .child(SharedString::from(format!(
                    "Could not read the batch: {message}"
                )))
                .into_any_element(),
            BatchLoad::Ready(records) => {
                let shown = records.iter().take(MAX_BATCH_ROWS);
                let hidden = records.len().saturating_sub(MAX_BATCH_ROWS);
                v_flex()
                    .gap(u(px(2.)))
                    .children(shown.map(|member| {
                        let this_one = member == record;
                        div()
                            .debug_selector(|| "audit-batch-row".into())
                            .font_weight(if this_one {
                                gpui::FontWeight::BOLD
                            } else {
                                gpui::FontWeight::NORMAL
                            })
                            .child(SharedString::from(format!(
                                "{} {} {}",
                                outcome_text(member.outcome),
                                member.target.gvk.kind,
                                member.target.name
                            )))
                    }))
                    .children((hidden > 0).then(|| {
                        div()
                            .text_color(tokens.colors.text_muted)
                            .child(SharedString::from(format!("and {hidden} more")))
                    }))
                    .into_any_element()
            }
        };
        let count = match &state.load {
            BatchLoad::Ready(records) if records.len() >= MAX_BATCH_RECORDS => {
                format!("Batch ({MAX_BATCH_RECORDS}+ records)")
            }
            BatchLoad::Ready(records) => format!("Batch ({} records)", records.len()),
            _ => "Batch".to_owned(),
        };
        Some(
            v_flex()
                .debug_selector(|| "audit-batch".into())
                .gap(u(px(4.)))
                .child(
                    div()
                        .text_size(u(tokens.font.small))
                        .text_color(tokens.colors.text_muted)
                        .child(SharedString::from(count)),
                )
                .child(div().text_size(u(tokens.font.small)).child(body))
                .into_any_element(),
        )
    }

    /// The panel, when a record is selected.
    pub(super) fn detail_panel(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let record = self.model(cx).selected_record()?.clone();
        let names = self.table.delegate(cx).names.clone();
        let tokens = cx.tokens();
        let mut reason = outcome_text(record.outcome).to_owned();
        if let Some(denial) = record.denial {
            reason = format!("{reason} ({})", denial_text(denial));
        }
        let fields = vec![
            self.field("Time (UTC)", record.ts.to_string(), cx),
            self.field("Cluster", names.name(&record.cluster), cx),
            self.field(
                "Namespace",
                cell_text(&record, AuditColumn::Namespace, &names),
                cx,
            ),
            self.field(
                "Object",
                format!("{}/{}", record.target.gvk.kind, record.target.name),
                cx,
            ),
            self.field("Command", record.cmd.to_string(), cx),
            self.field("Verb", cell_text(&record, AuditColumn::Verb, &names), cx),
            self.field("Outcome", reason, cx),
            self.field("Initiator", initiator_text(record.initiator), cx),
            self.field("Who", record.who.to_string(), cx),
            self.field(
                "Dry-run",
                cell_text(&record, AuditColumn::DryRun, &names),
                cx,
            ),
        ];
        let extras: Vec<AnyElement> = record
            .detail
            .as_ref()
            .map(|detail| self.field("Detail", detail.to_string(), cx))
            .into_iter()
            .collect();
        let batch = self.batch_section(&record, cx);
        Some(
            v_flex()
                .id("audit-detail")
                .debug_selector(|| "audit-detail".into())
                .flex_none()
                .w(u(px(300.)))
                .h_full()
                .gap(u(tokens.spacing.sm))
                .p(u(tokens.spacing.md))
                .border_l_1()
                .border_color(tokens.colors.border_variant)
                .overflow_y_scroll()
                .child(
                    h_flex()
                        .gap(u(tokens.spacing.sm))
                        .child(tagged(
                            "audit-show-resource",
                            Button::new("audit-show-resource")
                                .label("Show resource")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|view, _, _, cx| view.show_resource(cx))),
                        ))
                        .child(div().flex_1())
                        .child(tagged(
                            "audit-close-detail",
                            Button::new("audit-close-detail")
                                .label("Close")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(|view, _, _, cx| view.select(None, cx))),
                        )),
                )
                .children(fields)
                .children(extras)
                .children(batch)
                .into_any_element(),
        )
    }
}
