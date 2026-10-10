//! The `palette` scenario (E11-S03): the command palette over 2 000 registered commands
//! (docs/PERFORMANCE.md "Palette": open <= 1 frame, filter 2 000 entries <= 5 ms).
//!
//! It runs the real [`CommandPalette`] view over fixture commands (every category, view and
//! selection shape, `oxikube_testkit::commands`), classified for a table with a pod selected, in a
//! headless window built as the app builds its own ([`window_root`]).
//!
//! 1. **Open** (`open_ms`): from building the view (classifying the 2 000 commands, ordering the
//!    recents first, constructing the picker) to the end of the first frame that lists them.
//! 2. **Typing**: [`FRAMES`] scripted frames; each types the next character of a query (growing
//!    from one letter to a few words, then cleared), which matches the 2 000 commands (on the
//!    background executor above 512), and draws the frame that shows the matches. `frame_ms` is the
//!    per-frame cost through the frame hook, `draw_ms` the frame as drawn from outside.
//!
//! The sample fails when the view's text is drawn in a family the machine lacks (#509).

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context as _, Result, ensure};
use gpui::{AnyWindowHandle, AppContext as _, Entity, WeakEntity};
use oxikube_app::{
    CommandContext, CommandIndex, CommandInfo, CommandTarget, MemoryRecents, RecentsStore,
    Selection,
};
use oxikube_domain::Capabilities;
use oxikube_domain::command::{CommandId, ViewContext};
use oxikube_domain::ids::Gvk;
use oxikube_palette::CommandPalette;
use oxikube_palette::command_palette::{Outbox, PaletteParts, Snapshot};
use oxikube_runtime::perf::harness;
use oxikube_runtime::perf::{Recorder, ScenarioSample, Summary, round_ms};
use oxikube_testkit::commands::fixture_commands;
use oxikube_testkit::headless;

use super::{FRAMES, WINDOW_SIZE, window_root};

/// The scenario's name.
pub(super) const NAME: &str = "palette";

/// The scenario's own metric.
const OPEN_MS: &str = "open_ms";

/// Registered commands.
const COMMANDS: usize = 2_000;

/// What gets typed, one character per frame; then the field is cleared and it starts again.
const TYPED: &str = "fixture 19 pod";

/// One sample. `probe` puts the `--perf` frame hook in the window.
pub(super) fn run(probe: bool) -> Result<ScenarioSample> {
    let recorder = Arc::new(Recorder::new());
    oxikube_runtime::perf::install(recorder.clone());

    let mut cx = headless::headless_context_with_assets(Arc::new(oxikube_ui::Assets));
    cx.update(|cx| {
        oxikube_ui::init(cx);
        oxikube_ui::set_tokens(cx, oxikube_ui::Tokens::dark());
        oxikube_runtime::init_deterministic(cx);
    });
    let index = CommandIndex::new(
        fixture_commands(COMMANDS)
            .into_iter()
            .map(|meta| CommandInfo::new(meta, "perf", true)),
    )
    .context("the fixture commands have distinct ids")?;
    let mut context = CommandContext::new(ViewContext::Table)
        .with_capabilities(Capabilities::all())
        .selecting(Selection::one(Gvk::new("", "v1", "Pod")));
    context.cluster_active = true;
    let recents = Arc::new(MemoryRecents::new());
    for ix in [3, 40, 700] {
        recents.record(CommandId::new(Box::leak(
            format!("pod::Fx{}", ix * 8 + 4).into_boxed_str(),
        )));
    }

    // 1. Open.
    let hook = probe.then(|| recorder.clone());
    let started = Instant::now();
    let mut palette: Option<Entity<CommandPalette>> = None;
    let mut text_font = None;
    let window: AnyWindowHandle = cx
        .open_window(WINDOW_SIZE, |window, cx| {
            let parts = PaletteParts {
                snapshot: Snapshot::take(&index, &context),
                target: CommandTarget::none(),
                outbox: Outbox::default(),
                recents: recents.clone() as Arc<dyn RecentsStore>,
                workspace: WeakEntity::new_invalid(),
            };
            let view = cx.new(|cx| CommandPalette::new(parts, window, cx));
            palette = Some(view.clone());
            let (root, font) = window_root::mount(view.into(), hook, window, cx);
            text_font = Some(font);
            root
        })?
        .into();
    let palette = palette.context("the palette view")?;
    cx.run_until_parked();
    harness::draw_frame(&mut cx, window, |_, _| {})?;
    let open = started.elapsed();
    cx.update(|cx| text_font.context("the window root")?.check(cx))?;
    let listed = cx.update(|cx| palette.read(cx).picker().read(cx).delegate.listed().len());
    ensure!(listed > COMMANDS / 2, "{listed} commands listed on open");

    // 2. Typing.
    let mut reader = recorder.reader();
    reader.drain(&recorder);
    let picker = cx.update(|cx| palette.read(cx).picker().clone());
    let chars: Vec<char> = TYPED.chars().collect();
    let run = harness::run_frames(
        &mut cx,
        window,
        FRAMES,
        &recorder,
        &mut reader,
        |cx| cx.run_until_parked(),
        |frame, window, cx| {
            let typed = frame % (chars.len() + 1);
            let query: String = chars[..typed].iter().collect();
            picker.update(cx, |picker, cx| picker.set_query(&query, window, cx));
        },
    )?;

    let open_ms = round_ms(open.as_secs_f64() * 1000.0);
    eprintln!(
        "oxikube palette: opened {listed} commands in {open_ms} ms; {} frames",
        run.counters.frames
    );
    Ok(run.into_sample(NAME, [(OPEN_MS.to_owned(), Summary::single(open_ms))]))
}
