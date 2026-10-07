//! Clearing a terminal from the outside (`terminal::Clear`): scrollback gone, the prompt at the
//! top. The emulator is driven directly (its `Handler` methods), never through the parser, so a
//! half-received escape sequence of the process is not disturbed.

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{ClearMode, Handler as _};

use super::TermGrid;

impl TermGrid {
    /// Drops the scrollback and moves the lines from the cursor's line down to the top of the
    /// screen, clearing what was above (the shell's prompt stays where you type, like `clear`).
    /// The view goes back to the live screen and the selection is dropped. The process is not
    /// told. On the alternate screen (vim, htop) there is no scrollback and nothing changes.
    pub fn clear(&mut self) {
        if self.term.mode().contains(TermMode::ALT_SCREEN) {
            return;
        }
        let above = usize::try_from(self.term.grid().cursor.point.line.0).unwrap_or(0);
        if above > 0 {
            self.term.scroll_up(above);
            self.term.move_up(above);
        }
        self.term.clear_screen(ClearMode::Saved);
        self.term.scroll_display(Scroll::Bottom);
        self.term.selection = None;
        self.layout_generation += 1;
        self.listener.discard();
    }
}
