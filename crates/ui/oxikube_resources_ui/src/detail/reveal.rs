//! Revealing a Secret in its detail (E12-S11): `secret::Reveal`, `x`.
//!
//! A Secret's detail never holds a value by default: the full read removes them
//! ([`mask_secret`](super::model::mask_secret)) and keeps only a [`SecretSummary`] (key names,
//! sizes, the registries and certificate a type puts on display). Revealing reads the Secret
//! once more (`SecretService::read_decoded`, a `get` on the Kubernetes runtime) and decodes it in
//! memory; the decoded values live in this view's [`SecretState`] and nowhere else:
//!
//! * the reveal is per detail (a pinned tab and the drawer reveal on their own) and goes when the
//!   view is released (the tab closes), when it is toggled off, and after [`REVEAL_TIMEOUT`];
//! * the YAML tab shows the revealed keys as text while it lasts, and refuses to be saved to a
//!   file meanwhile (copy writes the clipboard the user asked for; nothing decoded reaches disk);
//! * a new version of the object reads the values again, so the view follows the object;
//! * the event is logged by the service (the object and the number of keys), never a value.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use gpui::{Context, SharedString, Task};
use oxikube_app::{SecretService, SecretSummary};
use oxikube_domain::OxiError;
use oxikube_domain::secret::{DecodedSecret, DecodedValue};
use oxikube_runtime::spawn_kube;

use super::model::is_secret;
use super::view::DetailView;

/// How long revealed values stay on screen before they are hidden again.
pub const REVEAL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// The most characters of a revealed value a row shows.
const SHOWN_CHARS: usize = 96;

/// How far the read behind a reveal is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum RevealLoad {
    /// Nothing in flight.
    #[default]
    Idle,
    /// The Secret is being read.
    Loading,
    /// The read failed (forbidden, gone): why.
    Failed(String),
}

/// What a detail view knows of a Secret: the summary, and what the user asked to see decoded.
#[derive(Default)]
pub(super) struct SecretState {
    /// Key names, sizes and the type's public facts, from the full read.
    pub(super) summary: Option<Arc<SecretSummary>>,
    /// Every key was asked for (`x`).
    all: bool,
    /// The keys asked for one by one.
    keys: BTreeSet<String>,
    /// The Secret decoded, while something is revealed. In memory only.
    decoded: Option<Arc<DecodedSecret>>,
    /// The one line a row shows for each decoded key, made when the Secret was read: a redraw
    /// copies a line, never a value.
    lines: HashMap<String, SharedString>,
    load: RevealLoad,
    /// Counts the changes of what is shown decoded: the YAML text is made again when it moves.
    pub(super) epoch: u64,
    /// The read in flight; a newer one replaces (and so cancels) it.
    task: Option<Task<()>>,
    /// The hide-again timer; replaced by each reveal, never cleared from inside.
    timer: Option<Task<()>>,
}

/// The decoded values the YAML tab shows, for the background thread that writes the text.
#[derive(Clone)]
pub(super) struct RevealedValues {
    pub(super) decoded: Arc<DecodedSecret>,
    /// The keys shown; `None` shows every key.
    pub(super) keys: Option<BTreeSet<String>>,
}

impl SecretState {
    fn requested(&self) -> bool {
        self.all || !self.keys.is_empty()
    }

    fn wants(&self, key: &str) -> bool {
        self.all || self.keys.contains(key)
    }

    /// Forgets what was asked for and what was read, keeping `load` (a failure is shown).
    fn forget_requests(&mut self) {
        self.all = false;
        self.keys.clear();
        self.decoded = None;
        self.lines.clear();
    }

    fn forget_values(&mut self) {
        self.forget_requests();
        self.load = RevealLoad::Idle;
        self.task = None;
        self.epoch += 1;
    }

    /// Drops everything the state holds (the view is going away).
    pub(super) fn clear(&mut self) {
        self.forget_values();
        self.summary = None;
        self.timer = None;
    }
}

impl DetailView {
    /// The Secret's key names, sizes and type facts, once its full read has landed.
    pub fn secret_summary(&self) -> Option<&Arc<SecretSummary>> {
        self.secret.summary.as_ref()
    }

    /// Whether this detail is of a Secret.
    pub fn is_secret(&self) -> bool {
        is_secret(&self.target.gvk)
    }

    /// Whether the user asked to see any value decoded (the read may still be in flight).
    pub fn reveal_requested(&self) -> bool {
        self.secret.requested()
    }

    /// Whether every key was asked for.
    pub fn reveals_all(&self) -> bool {
        self.secret.all
    }

    /// Whether the user asked to see `key` decoded.
    pub fn key_requested(&self, key: &str) -> bool {
        self.secret.wants(key)
    }

    /// The decoded value of `key`, while it is revealed and read.
    pub fn revealed_value(&self, key: &str) -> Option<&DecodedValue> {
        if !self.secret.wants(key) {
            return None;
        }
        self.secret.decoded.as_ref()?.get(key)
    }

    /// The one line a row shows for `key`, while it is revealed and read: cut and counted when the
    /// Secret was read, so drawing it costs a pointer copy.
    pub fn revealed_line(&self, key: &str) -> Option<&SharedString> {
        if !self.secret.wants(key) {
            return None;
        }
        self.secret.lines.get(key)
    }

    /// Whether decoded values are on screen now (the YAML tab shows them too).
    pub fn secret_values_shown(&self) -> bool {
        self.secret.requested() && self.secret.decoded.is_some()
    }

    /// How far the read behind the reveal is.
    pub fn reveal_load(&self) -> &RevealLoad {
        &self.secret.load
    }

    /// Reveals or hides decoded values (`secret::Reveal`, `x`): every key when `key` is `None`
    /// (hides everything when anything is shown), else that key. The Secret is read once, in
    /// memory, when something is revealed that is not yet.
    pub fn toggle_reveal(&mut self, key: Option<&str>, cx: &mut Context<Self>) {
        if !self.is_secret() {
            return;
        }
        match key {
            None if self.secret.requested() => self.hide_values(cx),
            None => {
                self.secret.all = true;
                self.revealed(cx);
            }
            Some(key) => {
                if self.secret.all {
                    // From "every key" to "every key but this one".
                    let others = self
                        .secret
                        .summary
                        .as_ref()
                        .map(|summary| {
                            summary
                                .keys
                                .iter()
                                .filter(|k| k.key != key)
                                .map(|k| k.key.clone())
                                .collect::<BTreeSet<_>>()
                        })
                        .unwrap_or_default();
                    self.secret.all = false;
                    self.secret.keys = others;
                } else if !self.secret.keys.remove(key) {
                    self.secret.keys.insert(key.to_owned());
                }
                if self.secret.requested() {
                    self.revealed(cx);
                } else {
                    self.hide_values(cx);
                }
            }
        }
    }

    /// Hides every decoded value and forgets them.
    pub fn hide_values(&mut self, cx: &mut Context<Self>) {
        if !self.secret.requested() && self.secret.decoded.is_none() {
            return;
        }
        self.secret.forget_values();
        self.drop_yaml_text(cx);
        self.refresh_yaml(cx);
        cx.notify();
    }

    /// Takes the YAML text off the screen at once (it may hold decoded values): the masked text
    /// replaces it when it is made.
    fn drop_yaml_text(&mut self, cx: &mut Context<Self>) {
        self.yaml.making = None;
        self.yaml.task = None;
        if self.yaml.text.take().is_some()
            && let Some(view) = &self.yaml.view
        {
            view.update(cx, |view, cx| view.clear(cx));
        }
    }

    /// Something is to be shown decoded: reads the Secret when the values are not held, and
    /// starts the hide-again timer.
    fn revealed(&mut self, cx: &mut Context<Self>) {
        self.secret.epoch += 1;
        self.secret.load = RevealLoad::Idle;
        if self.secret.decoded.is_none() {
            self.fetch_decoded(cx);
        }
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(REVEAL_TIMEOUT).await;
            this.update(cx, |view, cx| view.hide_values(cx)).ok();
        });
        self.secret.timer = Some(timer);
        self.refresh_yaml(cx);
        cx.notify();
    }

    /// Reads the Secret and decodes it, off the UI thread.
    pub(super) fn fetch_decoded(&mut self, cx: &mut Context<Self>) {
        let Some(reader) = self
            .deps
            .sessions
            .get(&self.target.cluster)
            .and_then(|session| session.resources())
        else {
            self.secret.load = RevealLoad::Failed("The cluster is not connected.".to_owned());
            self.secret.forget_requests();
            return;
        };
        self.secret.load = RevealLoad::Loading;
        let target = self.target.clone();
        let read = spawn_kube(cx, async move {
            SecretService::new().read_decoded(&*reader, &target).await
        });
        self.secret.task = Some(cx.spawn(async move |this, cx| {
            let result = match read.await {
                Ok(result) => result,
                Err(error) => Err(OxiError::from(error)),
            };
            this.update(cx, |view, cx| view.decoded_read(result, cx))
                .ok();
        }));
    }

    fn decoded_read(&mut self, result: Result<DecodedSecret, OxiError>, cx: &mut Context<Self>) {
        if !self.secret.requested() {
            // Hidden again while it was being read: the values are dropped unseen.
            return;
        }
        match result {
            Ok(decoded) => {
                self.secret.lines = decoded
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            entry.key.clone(),
                            entry.value.summary_line(SHOWN_CHARS).into(),
                        )
                    })
                    .collect();
                self.secret.decoded = Some(Arc::new(decoded));
                self.secret.load = RevealLoad::Idle;
            }
            Err(error) => {
                tracing::warn!(%error, target = %self.target, "revealing the Secret failed");
                self.secret.load = RevealLoad::Failed(error.message().to_owned());
                self.secret.forget_requests();
            }
        }
        self.secret.epoch += 1;
        self.refresh_yaml(cx);
        cx.notify();
    }

    /// The values the YAML tab shows decoded, if any.
    pub(super) fn revealed_values(&self) -> Option<RevealedValues> {
        if !self.secret.requested() {
            return None;
        }
        Some(RevealedValues {
            decoded: self.secret.decoded.clone()?,
            keys: (!self.secret.all).then(|| self.secret.keys.clone()),
        })
    }

    /// Drops the Secret state: the summary, the values, the timers (the view is going away).
    pub(super) fn clear_secret(&mut self) {
        let had_values = self.secret.requested() || self.secret.decoded.is_some();
        self.secret.clear();
        if had_values {
            // The YAML text may hold them.
            self.yaml.making = None;
            self.yaml.task = None;
            self.yaml.text = None;
            self.yaml.view = None;
        }
    }
}

impl DetailView {
    /// Sends `secret::Reveal` for this Secret (`key` for one key, `None` for all of them): the
    /// toolbar's toggle, the panel's buttons and the `x` key all end here, like the palette and
    /// an agent do.
    pub fn request_reveal(&mut self, key: Option<&str>, cx: &mut Context<Self>) {
        let command = oxikube_domain::command::Command::SecretReveal {
            target: self.target.clone(),
            key: key.map(str::to_owned),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }

    /// Sends `secret::EditDecoded` for this Secret: an editor tab with the values as text.
    pub fn request_edit_decoded(&mut self, cx: &mut Context<Self>) {
        let command = oxikube_domain::command::Command::SecretEditDecoded {
            target: self.target.clone(),
        };
        self.deps.dispatcher.dispatch(command, cx);
    }
}
