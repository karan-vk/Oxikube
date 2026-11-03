//! The installed font families the font inputs suggest: listed once per app, on the background
//! executor. Asking the platform for every family takes tens of milliseconds (CoreText walks
//! every face), far over a frame, so it never runs in `render`: the first page that shows a font
//! field starts the listing, the input works without suggestions until it arrives, and every page
//! opened later reads the cached list. (Zed keeps a font family cache for the same reason.)

use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Global, SharedString};

use super::state::Controls;
use crate::page::SettingsPage;

/// The app's list of installed font families, once listed.
struct InstalledFonts(Arc<[SharedString]>);

impl Global for InstalledFonts {}

/// The families `names` lists, as the inputs suggest them: no hidden (`.`-prefixed) system
/// families, sorted, each once.
pub(super) fn suggestable(names: Vec<String>) -> Arc<[SharedString]> {
    let mut names: Vec<SharedString> = names
        .into_iter()
        .filter(|name| !name.starts_with('.'))
        .map(SharedString::from)
        .collect();
    names.sort();
    names.dedup();
    names.into()
}

/// The installed families, if the app has listed them.
pub(super) fn cached(cx: &App) -> Option<Arc<[SharedString]>> {
    cx.try_global::<InstalledFonts>()
        .map(|fonts| fonts.0.clone())
}

impl Controls {
    /// Makes sure the page has, or is getting, the installed families: from the app's cache, else
    /// by listing them on the background executor (the page holds the task; dropping the page
    /// cancels it, and the next page lists them again). Never blocks.
    pub(super) fn request_fonts(&mut self, cx: &mut Context<SettingsPage>) {
        if self.fonts.is_some() || self.fonts_task.is_some() {
            return;
        }
        if let Some(fonts) = cached(cx) {
            self.fonts = Some(fonts);
            return;
        }
        let text_system = cx.text_system().clone();
        self.fonts_task = Some(cx.spawn(async move |page, cx| {
            let fonts = cx
                .background_spawn(async move { suggestable(text_system.all_font_names()) })
                .await;
            let _ = page.update(cx, |page, cx| {
                cx.set_global(InstalledFonts(fonts.clone()));
                page.controls.fonts = Some(fonts);
                cx.notify();
            });
        }));
    }

    /// Whether the installed families have arrived (the font inputs suggest from them).
    pub fn fonts_listed(&self) -> bool {
        self.fonts.is_some()
    }
}
