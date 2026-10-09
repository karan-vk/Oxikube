//! The same colour slot of two themes, side by side: what the glyph warm-up (`crate::glyph_warm`)
//! reads to learn which text-rendering levels a switch from one theme to the other draws at.

use super::ThemeTokens;
use super::colors::{StatusColor, StatusColors, TerminalColors};
use super::oxikube::OxikubeColors;
use gpui::Hsla;

impl ThemeTokens {
    /// Calls `f(mine, theirs)` for every colour slot the two themes both have: every interface,
    /// editor, terminal, status, VCS and `oxikube` colour, the players and accents both list (by
    /// index) and the syntax styles both set a colour for (by capture name).
    pub fn for_each_color_pair(&self, other: &ThemeTokens, mut f: impl FnMut(Hsla, Hsla)) {
        let (mine, theirs) = (self.fixed_colors(), other.fixed_colors());
        debug_assert_eq!(mine.len(), theirs.len());
        for (a, b) in mine.into_iter().zip(theirs) {
            f(a, b);
        }
        for (a, b) in self.players.iter().zip(&other.players) {
            f(a.cursor, b.cursor);
            f(a.background, b.background);
            f(a.selection, b.selection);
        }
        for (a, b) in self.accents.iter().zip(&other.accents) {
            f(*a, *b);
        }
        for (name, style) in &self.syntax.styles {
            let theirs = other.syntax.styles.get(name).and_then(|s| s.color);
            if let (Some(a), Some(b)) = (style.color, theirs) {
                f(a, b);
            }
        }
    }

    /// Every colour of the fixed-shape groups, in a fixed order (the same for every theme).
    fn fixed_colors(&self) -> Vec<Hsla> {
        let mut out = Vec::with_capacity(256);
        self.colors.push_colors(&mut out);
        self.editor.push_colors(&mut out);
        push_terminal(&self.terminal, &mut out);
        push_status(&self.status, &mut out);
        self.vcs.push_colors(&mut out);
        push_oxikube(&self.oxikube, &mut out);
        out
    }
}

fn push_terminal(t: &TerminalColors, out: &mut Vec<Hsla>) {
    out.extend([
        t.background,
        t.foreground,
        t.bright_foreground,
        t.dim_foreground,
        t.cursor,
        t.selection,
    ]);
    t.ansi.push_colors(out);
    t.bright.push_colors(out);
    t.dim.push_colors(out);
}

fn push_status(s: &StatusColors, out: &mut Vec<Hsla>) {
    let all: [&StatusColor; 14] = [
        &s.conflict,
        &s.created,
        &s.deleted,
        &s.error,
        &s.hidden,
        &s.hint,
        &s.ignored,
        &s.info,
        &s.modified,
        &s.predictive,
        &s.renamed,
        &s.success,
        &s.unreachable,
        &s.warning,
    ];
    for c in all {
        out.extend([c.foreground, c.background, c.border]);
    }
}

fn push_oxikube(o: &OxikubeColors, out: &mut Vec<Hsla>) {
    out.extend([
        o.status_running,
        o.status_pending,
        o.status_failed,
        o.status_succeeded,
        o.status_terminating,
        o.status_unknown,
    ]);
    out.extend(o.cluster_tabs);
    out.extend(o.log_sources);
}

#[cfg(test)]
mod tests {
    use crate::appearance::Appearance;
    use crate::tokens::ThemeTokens;

    #[test]
    fn pairs_the_same_slot_of_both_themes() {
        let dark = ThemeTokens::fallback(Appearance::Dark);
        let light = ThemeTokens::fallback(Appearance::Light);
        let mut pairs = Vec::new();
        dark.for_each_color_pair(light, |a, b| pairs.push((a, b)));
        assert!(pairs.contains(&(dark.colors.text, light.colors.text)));
        assert!(pairs.contains(&(dark.oxikube.status_running, light.oxikube.status_running)));
        assert!(pairs.contains(&(dark.terminal.ansi.green, light.terminal.ansi.green)));
        // A theme paired with itself pairs every colour with itself.
        dark.for_each_color_pair(dark, |a, b| assert_eq!(a, b));
    }
}
