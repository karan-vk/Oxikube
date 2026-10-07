//! A window with one `TerminalElement` over a real local PTY (E09-S05, E09-S06), to see the element
//! paint a live program and take keyboard, IME and mouse input before the terminal tab exists
//! (E09-S07).
//!
//! `cargo run -p oxikube_terminal --example terminal_preview [-- <program> [args...]]`
//!
//! Without arguments it prints a colour and attribute sample and then runs `top`, which redraws
//! the screen every second (cursor addressing, the alternate screen). Resize the window: the grid
//! reflows and the program gets the new size. Drag to select, double-click a word, scroll the
//! history with the wheel, hold cmd (ctrl on Linux) over a URL or path to see it underlined;
//! clicking it prints the `terminal::OpenLink` command it would dispatch.
//!
//! Type into it: arrows, function keys, Ctrl-letters and (on Linux, or with
//! `"terminal": {"option_as_meta": true}` elsewhere) Alt-as-meta go to the program; cmd-c / cmd-v
//! (ctrl-shift-c / ctrl-shift-v on Linux and Windows) copy and paste; an input method composes
//! inline at the cursor; `htop`, `vim` or `less` get mouse reports (Shift-drag selects instead).
//! The preview has no workspace, so a multi-line paste is not asked about.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::rc::Rc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, IntoElement, ParentElement as _,
    Render, Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use oxikube_domain::command::Command;
use oxikube_terminal::backend::local::{LocalPty, LocalPtyOptions};
use oxikube_terminal::input::{Copy, Paste};
use oxikube_terminal::{PathLinks, TerminalElement, TerminalElementState, TerminalState};
use oxikube_workspace::CommandDispatcher;

const SAMPLE: &str = r#"printf '\033[1mbold\033[0m \033[3mitalic\033[0m \033[4munderline\033[0m \033[4:3mcurly\033[0m \033[9mstrike\033[0m \033[7minverse\033[0m\n'; for i in 0 1 2 3 4 5 6 7; do printf "\033[4${i}m  \033[10${i}m  \033[0m"; done; printf '\n\033[38;2;255;120;0mtruecolour\033[0m  wide: \344\275\240\345\245\275  https://kubernetes.io/docs  src/main.rs:12:3\n'; sleep 2; exec top"#;

/// Prints the commands a click would dispatch (the app sends them to its bus).
struct PrintCommands;

impl CommandDispatcher for PrintCommands {
    fn dispatch(&self, command: Command, _: &mut App) {
        println!("dispatch: {command:?}");
    }
}

struct Preview {
    terminal: Entity<TerminalState>,
    state: TerminalElementState,
    focus: FocusHandle,
}

impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cwd = std::env::current_dir().ok();
        div().size_full().child(
            TerminalElement::new(&self.terminal, &self.state, &self.focus)
                .dispatcher(Rc::new(PrintCommands))
                .path_links(PathLinks::Local { base: cwd }),
        )
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (program, program_args) = match args.next() {
        Some(program) => (program, args.collect()),
        None => (
            "/bin/sh".to_owned(),
            vec!["-c".to_owned(), SAMPLE.to_owned()],
        ),
    };
    let options = LocalPtyOptions {
        shell: Some(program),
        args: program_args,
        ..LocalPtyOptions::default()
    };
    // `spawn` forks: done here, before the UI loop starts, never on the UI thread.
    let pty = match LocalPty::spawn(options.clone()) {
        Ok(pty) => pty,
        Err(error) => {
            eprintln!("could not start the program: {error}");
            return;
        }
    };

    gpui_platform::application()
        .with_assets(oxikube_ui::Assets)
        .run(move |cx: &mut App| {
            if let Err(error) = oxikube_runtime::init(cx) {
                eprintln!("no tokio runtime: {error}");
                cx.quit();
                return;
            }
            oxikube_ui::init(cx);
            oxikube_theme::init_with_dir(None, cx);
            let (copy, paste) = if cfg!(target_os = "macos") {
                ("cmd-c", "cmd-v")
            } else {
                ("ctrl-shift-c", "ctrl-shift-v")
            };
            cx.bind_keys([
                gpui::KeyBinding::new(copy, Copy, Some("Terminal")),
                gpui::KeyBinding::new(paste, Paste, Some("Terminal")),
            ]);
            let bounds = Bounds::centered(None, size(px(900.), px(560.)), cx);
            let opened = cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..WindowOptions::default()
                },
                |window, cx| {
                    let terminal = cx.new(|cx| TerminalState::new(Box::new(pty), options.size, cx));
                    let focus = cx.focus_handle();
                    window.focus(&focus, cx);
                    cx.new(|cx| {
                        // Repaint when the terminal changes (at most once a frame).
                        cx.observe(&terminal, |_, _, cx| cx.notify()).detach();
                        Preview {
                            terminal,
                            state: TerminalElementState::new(),
                            focus,
                        }
                    })
                },
            );
            if let Err(error) = opened {
                eprintln!("could not open the window: {error}");
                cx.quit();
            }
            cx.activate(true);
        });
}
