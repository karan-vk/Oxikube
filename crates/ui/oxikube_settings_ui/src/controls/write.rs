//! Getting a valid value into `settings.json`: the debounce, one write in flight per field, and
//! reset.

use gpui::{Context, SharedString};
use oxikube_domain::ids::ClusterId;
use oxikube_settings::update_user_settings_path;
use serde_json::Value;

use super::WRITE_DEBOUNCE;
use super::spec::ControlSpec;
use super::state::Controls;
use super::validate::{FieldError, validate_text};
use crate::page::SettingsPage;

impl Controls {
    /// The text of a text-like control changed: validate it, then write it after the debounce
    /// (or show why it cannot be written).
    pub(crate) fn typed(
        &mut self,
        path: &SharedString,
        spec: &ControlSpec,
        text: &str,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        match validate_text(spec, text) {
            Ok(value) => self.accept(path, value, cluster, false, cx),
            Err(error) => self.reject(path, error, cx),
        }
    }

    /// A switch or a choice was used: write `value` at once.
    pub(crate) fn chose(
        &mut self,
        path: &SharedString,
        value: Value,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        self.accept(path, value, cluster, true, cx);
    }

    /// `value` is acceptable for `path`: write it now or after the debounce.
    pub(super) fn accept(
        &mut self,
        path: &SharedString,
        value: Value,
        cluster: Option<ClusterId>,
        immediately: bool,
        cx: &mut Context<SettingsPage>,
    ) {
        self.clear_error(path);
        if immediately {
            self.pending.remove(path);
            self.dirty.remove(path);
            self.write(path, value, cluster, cx);
        } else {
            self.schedule(path, value, cluster, cx);
        }
        cx.notify();
    }

    /// The input for `path` is not acceptable: show `error`, write nothing, and drop any older
    /// value still waiting (it is stale now).
    pub(super) fn reject(
        &mut self,
        path: &SharedString,
        error: FieldError,
        cx: &mut Context<SettingsPage>,
    ) {
        self.pending.remove(path);
        self.dirty.remove(path);
        self.errors.insert(path.clone(), error);
        cx.notify();
    }

    /// Removes the user's value for `path` so the default shows through.
    pub fn reset(
        &mut self,
        path: &SharedString,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        self.pending.remove(path);
        self.dirty.remove(path);
        self.errors.remove(path);
        self.resync.insert(path.clone());
        self.write(path, Value::Null, cluster, cx);
        cx.notify();
    }

    /// Starts the debounce timer of `path`; a newer value replaces the timer (and so cancels
    /// it) from outside, never from within the task.
    fn schedule(
        &mut self,
        path: &SharedString,
        value: Value,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        self.dirty.insert(path.clone());
        let key = path.clone();
        let timer = cx.spawn(async move |page, cx| {
            cx.background_executor().timer(WRITE_DEBOUNCE).await;
            page.update(cx, |page, cx| {
                page.controls.dirty.remove(&key);
                page.controls.write(&key, value, cluster, cx);
            })
            .ok();
        });
        self.pending.insert(path.clone(), timer);
    }

    /// Writes `value` at `path`, or queues it behind the write already in flight for the field.
    fn write(
        &mut self,
        path: &SharedString,
        value: Value,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        if self.in_flight.contains(path) {
            self.queued.insert(path.clone(), value);
            return;
        }
        self.in_flight.insert(path.clone());
        self.writes.retain(|task| !task.is_ready());
        let key_path: Vec<&str> = path.split('.').collect();
        let task = update_user_settings_path(cx, cluster.clone(), &key_path, value);
        let path = path.clone();
        let follow = cx.spawn(async move |page, cx| {
            let result = task.await;
            page.update(cx, |page, cx| {
                page.controls
                    .finished(&path, result.err().map(|err| err.to_string()), cluster, cx);
            })
            .ok();
        });
        self.writes.push(follow);
    }

    /// A write ended: show its failure, then send the value that waited behind it.
    fn finished(
        &mut self,
        path: &SharedString,
        failure: Option<String>,
        cluster: Option<ClusterId>,
        cx: &mut Context<SettingsPage>,
    ) {
        self.in_flight.remove(path);
        if let Some(message) = failure {
            self.errors.insert(
                path.clone(),
                FieldError(format!("Could not save: {message}")),
            );
        }
        if let Some(next) = self.queued.remove(path) {
            self.write(path, next, cluster, cx);
        }
        cx.notify();
    }
}
