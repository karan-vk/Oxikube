# Writing deterministic GPUI tests

Every UI story ships `#[gpui::test]`s (non-negotiable 10). A GPUI test is only useful if it fails
for the same reason every time, on macOS and on Linux CI. This page is the shared recipe: the
harness (`oxikube_testkit::gpui_test`, E05-S11), one worked example per helper, the rules that keep
tests deterministic, and the things that look harmless and are not.

The worked examples are real, passing tests: `crates/testing/oxikube_testkit/tests/gpui_harness.rs`
(deterministic tests), `tests/gpui_screenshot.rs` (screenshot + golden) and
`bins/oxikube/src/app_state/harness_tests.rs` (the harness over the real `AppState`). The view
they drive is `tests/support/mod.rs`. Copy them.

## Setup

In the crate under test:

```toml
[dev-dependencies]
gpui = { workspace = true, features = ["test-support"] }
oxikube_testkit = { workspace = true, features = ["gpui-test"] }   # + "gpui-screenshot" for pictures
```

| Testkit feature | Gives | Needs |
|---|---|---|
| `gpui-test` | `TestApp`, `TestWindow` (deterministic, libtest threads, every OS) | nothing |
| `gpui-headless` | `headless::headless_context()` (real text system, headless renderer) | main thread on macOS; GPU only to capture |
| `gpui-screenshot` | `ScreenshotApp`, `run_golden_cases`, `capture_view`; implies the two above and `screenshot` | a GPU device: Metal, or Vulkan (Mesa lavapipe) on Linux |

## The helpers

All of these run the tasks they made runnable before they return, so the next line of the test
sees the settled state.

### Open a window: `TestApp::new`, `open_window`

```rust
#[gpui::test]
fn open_window_shows_the_root_view(cx: &mut TestAppContext) {
    let mut app = TestApp::new(cx);
    app.bind_keys(bindings());
    let mut window = app.open_window(Counter::new);            // activated, settled
    assert_eq!(window.read_root(|counter, _| counter.count), 0);
}
```

`TestWindow` also derefs to gpui's `VisualTestContext`, so mouse events, `simulate_resize` and the
rest of gpui's API are one dot away. `update_root`, `read_root` and `root()` reach the view.

### Open a workspace: `open_workspace`

A workspace window (`Workspace` inside `Root`, with the `oxikube_ui` globals, the workspace /
modal / toast actions and the test-item builder registered) lives above the testkit's layer, so
its helper sits in the crate that owns it:

```rust
use oxikube_workspace::test_support::{TestItem, open_workspace};

#[gpui::test]
fn opens_an_item(cx: &mut TestAppContext) {
    let (workspace, mut vcx) = open_workspace(cx);              // vcx: VisualTestContext
    vcx.update(|window, cx| {
        let item = TestItem::build("Pods", cx);
        workspace.update(cx, |ws, cx| ws.open_item(item, window, cx));
    });
    vcx.run_until_parked();
}
```

`vcx` has the same verbs as `TestWindow` (`simulate_keystrokes`, `dispatch_action`,
`run_until_parked`). Call `open_workspace` again for a second window in the same app.

### Simulate keystrokes: `bind_keys`, `simulate_keystrokes`

```rust
app.bind_keys([KeyBinding::new("j", Increment, Some("Counter"))]);
window.simulate_keystrokes("j j j");     // space separated: "cmd-k escape"
```

Drive keymaps through keystrokes rather than calling handlers: that tests the binding, the key
context and the focus path. A keystroke reaches the focused element only, so focus something in the
view's `new` (`window.focus(&handle, cx)`). Real bindings come from `oxikube_keymap`; tests that
care about the shipped keymap load it, tests of a single view bind their own.

### Dispatch an action: `dispatch_action`

```rust
window.dispatch_action(Increment);       // what the command palette and menus do
```

### Advance the clock: `advance_clock`, `run_until_parked`

```rust
window.simulate_keystrokes("j");
app.run_until_parked();                          // tasks run; the 300 ms timer is still pending
assert!(!window.read_root(|c, _| c.settled));
app.advance_clock(Duration::from_millis(300));   // the timer fires
assert!(window.read_root(|c, _| c.settled));
```

`run_until_parked` runs everything that can run now. `advance_clock` moves the test clock, which is
what `cx.background_executor().timer(..)` and `cx.spawn` + `Timer` wait on. Never sleep.

### Fake ports and `AppState`: `TestPorts`, `AppState::test_with`

`oxikube_testkit::TestPorts::seeded()` bundles in-memory `FakeStatePort`, `FakeSecretStorePort`,
`FakeClusterSourcePort` (one kubeconfig source, context `kind-oxikube`) and `FakeResourcePort`
(a running and a crash-looping pod, a Deployment, a node, a namespace), sharing one
`FakeClockPort`. In `bins/oxikube` (feature `test-support`, always on in its own tests):

```rust
let ports = TestPorts::seeded();
let state = app.update(|cx| AppState::test_with(cx, &ports));   // the real init order, on fakes
ports.state.recorded_calls();                                    // assert on what the view did
```

`AppState::test(cx)` is the same without the handles. Both use the deterministic runtime and an
in-memory settings store with no file watchers. Other crates that need the app's `AppState` depend
on `oxikube` with `test-support`; crates that only need ports use `TestPorts` directly. The fakes'
clock (`FakeClockPort::advance`) and GPUI's clock (`advance_clock`) are separate: advance the one
that drives what you are waiting for.

### Drive the real window with keys: the keyboard suite

When the thing under test is "press these keys and the app does X" (the command palette, the `:`
jump bar, a base keymap), a view over stand-ins is not enough: the binding, key context, focus,
modal layer, `CommandBus` and the view that ends up on screen all have to agree. The app's own
main window over fakes (`mount/tests`, `App::start`) is the harness, and
`bins/oxikube/src/mount/tests/keyboard/` is the worked example (E11-S12): `App::keyboard(cx, vim)`
starts it over a fixture cluster with its Pods table focused; the scenarios press keys with
`app.press(..)` / `app.type_text(..)` and assert on the audit log, the tables the tab shows and the
calls the cluster port saw. `check!(app, cond, "..")` appends `app.diagnostics()` (focused view,
key-context stack, modal, cursor) to a failed assertion so a keymap regression names its context.
Settings edits (`base_keymap`) go through `SettingsStore::set_user_settings`, the path a hot
reload takes. Run with `cargo test -p oxikube keyboard`.

### Take a screenshot and compare it with a golden: `ScreenshotApp`

Pictures need the real text system and a GPU renderer, which `TestAppContext` does not have, so a
screenshot test is a separate `[[test]]` with `harness = false` (on macOS the platform text system
can only be created on the process main thread, which libtest workers are not):

```toml
[[test]]
name = "gpui_screenshot"
harness = false
required-features = ["gpui-golden"]     # a feature of your own that enables gpui-screenshot (see below)
```

Do not point `required-features` at `gpui-screenshot` itself in a crate that other crates' dev-deps
also enable it for: `cargo test --workspace` unifies features, so the GPU test would run in the PR
gate on runners without a GPU. The testkit's own example uses the dedicated `gpui-golden` feature
(`cargo test -p oxikube_testkit --features gpui-golden`); a crate gives its screenshot test a
feature of its own, like `oxikube_workspace`'s `screenshot`, and the nightly job turns it on.

```rust
fn render() -> anyhow::Result<RgbaImage> {
    let mut app = ScreenshotApp::new();                  // or with_assets(Arc::new(oxikube_ui::Assets))
    app.bind_keys(bindings());
    let window = app.open_window(size(px(320.), px(120.)), |window, cx| cx.new(|cx| Counter::new(window, cx)))?;
    app.simulate_keystrokes(window, "j j j")?;
    app.advance_clock(SETTLE_AFTER);
    app.capture(window)                                  // size * 2 pixels
}

fn main() -> ExitCode {
    run_golden_cases(env!("CARGO_MANIFEST_DIR"), &[GoldenCase { name: "counter_settled", size: (320, 120), render }])
}
```

`run_golden_cases` checks the image size, that the frame is not blank, and (when a golden exists
for this OS) that it matches within `Tolerance::default()` (8 per channel, 0.1% of pixels).

Goldens live in `<crate>/tests/goldens/<os>/<name>.png` and are tracked, because glyph rasterisation
differs per OS. To create or refresh one, run the test with `OXIKUBE_UPDATE_GOLDENS=1`, open the
PNG, and commit it:

```
OXIKUBE_UPDATE_GOLDENS=1 cargo test -p <crate> --features <your screenshot feature> --test <name>
```

An OS with no golden gets the structural checks only (and says so); add its golden by running the
same command on that OS (the nightly job uploads `*.actual.png` / `*.diff.png` on a mismatch, and
`*.actual.png` is the file to commit after reviewing it).

**Linux goldens** must come from the nightly's own runner image (ubuntu-latest, Mesa lavapipe, its
font set), not from a container on your machine. Refresh them with the `refresh-goldens` workflow,
from the repo root:

```
gh workflow run refresh-goldens.yml --ref <your-branch>
gh run list --workflow refresh-goldens.yml --branch <your-branch> --limit 1   # the run id
gh run download <run-id> -n linux-goldens        # drops the PNGs into their crates' tests/goldens/linux/
```

The workflow writes every golden in update mode and then runs the same tests again in compare mode,
so a golden that does not reproduce on its own runner fails the run instead of being committed.
Review the PNGs (a refresh accepts whatever the code draws today), commit them, and add a new
screenshot test to the workflow's test list (and to `nightly.yml`) so the same test refreshes and
compares its golden. Once a Linux golden is committed the nightly Linux leg runs `check_golden`
for that test and fails on a pixel regression, uploading `*.actual.png` / `*.diff.png`. The goldens
the Linux leg must have are listed in `xtask/src/workflows.rs` (`LINUX_GOLDENS`), which fails
`cargo test --workspace` when one is missing.

Pin everything that varies between machines before drawing: the theme (`oxikube_ui::set_tokens`),
reduce motion, window size, fonts.

## Rules that keep a test deterministic

1. **No OS threads.** A file watcher, a `std::thread::spawn`, a tokio runtime or a timer thread that
   wakes a GPUI task makes the scheduler panic with "Detected activity on thread ..., your test is
   not deterministic". It only shows on Linux and Windows, so it passes on a Mac and fails in CI.
   Constructors that start watchers take a `watch: false` flag (`ConfigSource::Memory`,
   `RuntimeChoice::Deterministic`); tests use them. `AppState::test` already does.
2. **No sleeping.** Not `std::thread::sleep`, not `tokio::time::sleep` on real time. Move the clock
   with `advance_clock`; wait for tasks with `run_until_parked`.
3. **No self-dropping tasks.** A `Task` stored in a field and cleared from inside itself cancels
   itself mid-flight. Store it from the outside (`self.task = Some(cx.spawn(..))`) and let the next
   call replace it, or use a done flag and `.detach()`. In tests, keep the `Task` you spawn alive
   until you await it: dropping it cancels the work and the test then "passes" without running it.
4. **No real network or disk.** Ports are fakes (`TestPorts`); Kubernetes work goes through
   `spawn_kube`, which in tests runs on GPUI's deterministic executor.
5. **Drive input like a user.** Keystrokes through the keymap, actions through
   `dispatch_action`; assert on the view's state or on the fakes' recorded calls, not on private
   handlers.
6. **Seed everything random or time based.** `#[gpui::test]` seeds the scheduler from `SEED` (default
   0); `#[gpui::test(iterations = 20)]` re-runs a test with different seeds to shake out ordering
   bugs; set `SEED=<n>` to reproduce a failing one. `FakeClockPort` starts at a fixed instant.
7. **Leaks fail the test.** `gpui/test-support` turns on leak detection: entities still alive when
   the test app shuts down are reported with the backtrace of their creation. Fix the cycle (a
   task or subscription holding a strong `Entity` of its owner: hold a `WeakEntity` instead).
8. **Do not paint what you do not check.** Plain `TestApp` tests skip the GPU. Only screenshot
   tests render pixels, and they run in the nightly job (they need a GPU), not in the PR gate.

## Don't: a non-deterministic example

This is documented, not committed, because it is flaky by construction:

```rust
// DON'T. A foreign thread wakes a GPUI task: Linux/Windows panic "Detected activity on thread ...".
#[gpui::test]
fn bad(cx: &mut TestAppContext) {
    let task = cx.spawn(async |cx| { /* wait for the thread's result */ });
    std::thread::spawn(move || { std::thread::sleep(Duration::from_millis(10)); tx.send(()).ok(); });
    //                           ^^^^^^^^^^^^^^^^^^ real time           ^^^^^^^^^ wakes the task off-thread
    task.await;
}
```

Do this instead: make the producer a fake that is driven by the test (`FakeClockPort` +
`Timeline` for streams, a `futures::channel::mpsc` the test sends on from the test thread), then
`run_until_parked`.

For a view over a live resource feed (a table, a detail) script the feed with
`oxikube_testkit::ScriptedFeed`: the initial list, then `add` / `modify` / `delete` at ticks, one
`DeltaBatch` per tick. `feed.install(&ports.resources)` before the cluster connects, then
`ports.resources.clock().advance(TICK)` and settle per tick; count a view's redraws with
`cx.observe(&view, ..)` to assert one redraw per batch. The worked examples are the scenarios in
`crates/ui/oxikube_resources_ui/src/suite/` (`Scripted` is the harness around the table fixture).

## Where the CI runs them

| Job | What | OS |
|---|---|---|
| `ci` / `clippy + test` | `cargo test --workspace` (every `#[gpui::test]`) and `cargo test -p oxikube_testkit --features gpui-test` | macOS and Linux |
| `nightly` / screenshot step | `cargo test -p oxikube_testkit --features gpui-golden --test gpui_screenshot` and the other golden tests | macOS (Metal), Linux (lavapipe under `xvfb-run`) |

The suite's own time is reported by `cargo test` (`finished in ...s`); opening a window in a
`TestApp` takes milliseconds, so keep tests small and one behaviour each.

## Checklist for a new UI test

- [ ] `TestApp::new(cx)` (or `open_workspace(cx)`), no threads, no sleeps.
- [ ] Keys bound with `bind_keys` and driven with `simulate_keystrokes`.
- [ ] Time moved with `advance_clock`, tasks settled with `run_until_parked`.
- [ ] State read back with `read_root` / the fakes' `recorded_calls()`.
- [ ] A screenshot test only when the result is visual, with a golden for macOS.
