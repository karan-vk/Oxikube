//! [`JumpDelegate`]: the picker behind the `:` bar. The query is the line; the matches are what
//! the word under the caret could be (aliases, namespaces, contexts); confirming runs the line.
//!
//! Each keystroke parses the line (cheap, on the UI thread) and matches the candidates of the
//! word being typed: inline for a few hundred, on the background executor above that, so the bar
//! never holds a frame. Enter plans the line against the cluster ([`oxikube_app::search::jump`]);
//! a line that names something that does not exist stays in the bar with the problem underlined
//! and close names offered, a good one closes the bar and its commands go out through the window's
//! dispatcher. Enter on a completion the user picked with the arrows or the mouse accepts that
//! completion instead of running the line.

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Context, DismissEvent, InteractiveElement as _, ParentElement as _,
    SharedString, Styled as _, Task, Window,
};
use oxikube_app::search::jump::{
    self, Candidate, CompletionSite, ParseError, ParseErrorKind, Slot, accept,
};
use oxikube_ui::layout::h_flex;

use super::host::Shared;
use super::render;
use super::sources::LiveEnv;
use crate::picker::fuzzy::{self, StringMatch, StringMatchCandidate};
use crate::picker::{Picker, PickerDelegate, match_label};

/// The most completions listed. The list is virtualised; the cap only bounds the work.
const MAX_MATCHES: usize = 1_000;

/// The candidates of one kind of word, ready to match.
struct Pool {
    slot: Option<Slot>,
    items: Arc<[Candidate]>,
    strings: Arc<[StringMatchCandidate]>,
}

impl Pool {
    fn empty() -> Self {
        Self {
            slot: None,
            items: Arc::new([]),
            strings: Arc::new([]),
        }
    }

    fn build(slot: Slot, env: &LiveEnv) -> Self {
        let items: Arc<[Candidate]> = jump::candidates(slot, env).into();
        let strings = items
            .iter()
            .enumerate()
            .map(|(id, candidate)| {
                StringMatchCandidate::new(id, SharedString::from(candidate.text.to_string()))
            })
            .collect();
        Self {
            slot: Some(slot),
            items,
            strings,
        }
    }
}

/// The delegate of the jump bar's picker.
pub struct JumpDelegate {
    env: Rc<LiveEnv>,
    shared: Rc<Shared>,
    /// The line as typed.
    line: String,
    pool: Pool,
    matches: Vec<StringMatch>,
    selected: usize,
    /// Whether the user chose a completion (arrows, click) since the last keystroke.
    picked: bool,
    /// What is wrong with the line so far, shown quietly while typing.
    syntax: Option<ParseError>,
    /// What Enter found wrong: shown loudly until the next keystroke.
    error: Option<ParseError>,
    /// Tab was pressed while the matches of the newest line were still being computed: take the
    /// selected completion as soon as they land, as Enter waits for them.
    complete_when_matched: bool,
}

impl JumpDelegate {
    pub(super) fn new(env: Rc<LiveEnv>, shared: Rc<Shared>) -> Self {
        Self {
            env,
            shared,
            line: String::new(),
            pool: Pool::empty(),
            matches: Vec::new(),
            selected: 0,
            picked: false,
            syntax: None,
            error: None,
            complete_when_matched: false,
        }
    }

    /// Asks for the selected completion to be taken once the matches of the newest line arrive
    /// (Tab while they are still being computed).
    pub(super) fn complete_when_matched(&mut self) {
        self.complete_when_matched = true;
    }

    /// The line as typed.
    pub fn line(&self) -> &str {
        &self.line
    }

    /// What Enter found wrong with the line, until the next keystroke.
    pub fn error(&self) -> Option<&ParseError> {
        self.error.as_ref()
    }

    /// What is wrong with the line so far (shown quietly while the user types).
    pub fn syntax_error(&self) -> Option<&ParseError> {
        self.syntax.as_ref()
    }

    /// The completions listed, best first.
    pub fn completions(&self) -> Vec<String> {
        self.matches.iter().map(|m| m.string.to_string()).collect()
    }

    /// The selected completion, the line and the word it would replace.
    pub fn completion(&self) -> Option<(String, CompletionSite, Arc<str>)> {
        let found = self.matches.get(self.selected)?;
        let candidate = self.pool.items.get(found.candidate_id)?;
        Some((
            self.line.clone(),
            jump::site(&self.line),
            candidate.text.clone(),
        ))
    }

    /// The data the bar was waiting for arrived (the contexts, the namespaces): plan against it
    /// from now on. The candidates are rebuilt on the next match update.
    pub(super) fn set_env(&mut self, env: Rc<LiveEnv>) {
        self.env = env;
        self.pool = Pool::empty();
    }

    fn match_now(&mut self, prefix: &str) {
        self.matches = fuzzy::match_strings(&self.pool.strings, prefix, MAX_MATCHES);
        self.selected = 0;
    }
}

impl PickerDelegate for JumpDelegate {
    type ListItem = gpui::Div;

    fn match_count(&self) -> usize {
        self.matches.len()
    }

    fn selected_index(&self) -> usize {
        self.selected
    }

    fn set_selected_index(&mut self, ix: usize, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.selected = ix;
        self.picked = true;
    }

    fn placeholder_text(&self, _: &mut Window, _: &mut App) -> SharedString {
        "Jump: pods, deploy kube-system, pod /re app=x @prod, ctx, ns, q".into()
    }

    fn no_matches_text(&self, _: &mut Window, _: &mut App) -> Option<SharedString> {
        None
    }

    fn update_matches(
        &mut self,
        query: String,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Task<()> {
        self.error = None;
        self.picked = false;
        self.syntax = jump::parse(&query)
            .err()
            .filter(|e| e.kind != ParseErrorKind::Empty);
        let site = jump::site(&query);
        self.line = query;
        if self.pool.slot != Some(site.slot) {
            self.pool = Pool::build(site.slot, &self.env);
        }
        // A blank prefix lists everything in order, and a small set matches in microseconds: both
        // in this frame. A long list with something typed matches on the background executor.
        if site.prefix.is_empty() || self.pool.strings.len() <= fuzzy::INLINE_MATCH_LIMIT {
            self.complete_when_matched = false;
            self.match_now(&site.prefix);
            return Task::ready(());
        }
        let strings = self.pool.strings.clone();
        let prefix = site.prefix;
        cx.spawn_in(window, async move |picker, cx| {
            let executor = cx.background_executor().clone();
            let matches = fuzzy::match_strings_async(strings, prefix, MAX_MATCHES, &executor).await;
            picker
                .update_in(cx, |picker, window, cx| {
                    picker.delegate.matches = matches;
                    picker.delegate.selected = 0;
                    // An arrow pressed while these were pending chose from the list they
                    // replace: nothing here has been chosen.
                    picker.delegate.picked = false;
                    if std::mem::take(&mut picker.delegate.complete_when_matched)
                        && let Some((line, site, text)) = picker.delegate.completion()
                    {
                        // After this task: setting the query replaces (and so drops) it.
                        let line = accept(&line, &site, &text);
                        let weak = cx.weak_entity();
                        window.defer(cx, move |window, cx| {
                            weak.update(cx, |picker, cx| picker.set_query(&line, window, cx))
                                .ok();
                        });
                    }
                })
                .ok();
        })
    }

    fn confirm(&mut self, _secondary: bool, window: &mut Window, cx: &mut Context<Picker<Self>>) {
        if self.picked
            && let Some((line, site, text)) = self.completion()
        {
            // The user chose a completion: take it, and keep typing.
            let line = accept(&line, &site, &text);
            let picker = cx.weak_entity();
            window.defer(cx, move |window, cx| {
                picker
                    .update(cx, |picker, cx| picker.set_query(&line, window, cx))
                    .ok();
            });
            return;
        }
        if self.line.trim().is_empty() {
            cx.emit(DismissEvent);
            return;
        }
        match jump::plan(&self.line, &*self.env) {
            Ok(plan) => {
                self.shared.submit(plan);
                cx.emit(DismissEvent);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn dismissed(&mut self, _: &mut Window, _: &mut Context<Picker<Self>>) {}

    fn render_match(
        &self,
        ix: usize,
        selected: bool,
        _: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> Option<Self::ListItem> {
        let found = self.matches.get(ix)?;
        let candidate = self.pool.items.get(found.candidate_id)?;
        Some(
            h_flex()
                .w_full()
                .justify_between()
                .gap_3()
                .debug_selector(move || format!("jump-item-{ix}"))
                .child(match_label(
                    found.string.clone(),
                    &found.positions,
                    selected,
                    cx,
                ))
                .child(render::detail(candidate.detail.clone(), cx)),
        )
    }

    fn render_header(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        render::header(&self.line, self.error.as_ref(), self.syntax.as_ref(), cx)
    }

    fn render_footer(&self, _: &mut Window, cx: &mut Context<Picker<Self>>) -> Option<AnyElement> {
        Some(render::footer(cx))
    }
}
