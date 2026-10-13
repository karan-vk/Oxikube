//! The per-platform code behind [`Backend`]: connect to the notification service, show a
//! notification, wait for the user. [`OsNotifier`](crate::OsNotifier) owns the policy (opt-in,
//! threads, replacement ids, click routing); a backend only talks to the OS and is blocking.

use std::sync::Arc;

use crate::payload::Payload;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", all(unix, not(target_os = "macos")))))]
mod none;
#[cfg(all(unix, not(target_os = "macos")))]
mod xdg;

/// Why a backend call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// There is no notification service to talk to (no daemon, unsupported platform). The
    /// reason is a short sentence.
    Unavailable(String),
    /// The service is there and refused or failed.
    Failed(String),
}

/// What the user did with a shown notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// Clicked the notification (or its default action).
    Clicked,
    /// Dismissed, expired or closed without an action.
    Closed,
}

/// A shown (or about-to-be-shown) notification. Blocking: run it on a thread of its own.
pub trait Shown: Send {
    /// The platform id, where there is one; a later notification with the same tag replaces
    /// this one by passing it as [`Payload::replaces`].
    fn id(&self) -> Option<u32>;

    /// Blocks until the user clicks or dismisses the notification. On macOS this is also what
    /// delivers it, so a failure there is logged and answered with [`Response::Closed`].
    fn wait(self: Box<Self>) -> Response;

    /// Delivers the notification without waiting for the user (the click is not reported).
    fn detach(self: Box<Self>);
}

/// The OS side of the notifier. Every method blocks and is called off the UI thread.
pub trait Backend: Send + Sync + 'static {
    /// Connects to the notification service, and asks for permission where the OS has such a
    /// thing. Called only when the user turns OS notifications on, and again after a failure.
    ///
    /// # Errors
    ///
    /// [`BackendError::Unavailable`] when there is no service.
    fn connect(&self) -> Result<(), BackendError>;

    /// Shows `payload`. `clickable` says whether the caller will [`wait`](Shown::wait) for a
    /// click (so a backend may add the action a click needs).
    ///
    /// # Errors
    ///
    /// [`BackendError::Unavailable`] when the service went away, [`BackendError::Failed`]
    /// when it refused.
    fn show(&self, payload: &Payload, clickable: bool) -> Result<Box<dyn Shown>, BackendError>;
}

/// The backend of the platform this crate was built for.
pub fn platform() -> Arc<dyn Backend> {
    #[cfg(target_os = "macos")]
    return Arc::new(macos::Macos::new());
    #[cfg(all(unix, not(target_os = "macos")))]
    return Arc::new(xdg::Xdg::new());
    #[cfg(not(any(target_os = "macos", all(unix, not(target_os = "macos")))))]
    return Arc::new(none::Unsupported);
}

/// The identifier (`CFBundleIdentifier`) of the `.app` bundle `exe` runs from
/// (`/Applications/Oxikube.app/Contents/MacOS/oxikube` reads
/// `/Applications/Oxikube.app/Contents/Info.plist`), if it runs from one and the plist is the
/// usual XML one. A plain read of a file: nothing here asks the OS (the lookup `mac-notification-sys`
/// offers by application name runs AppleScript, which opens a "Choose Application" dialog when the
/// name is not found).
#[cfg(any(target_os = "macos", test))]
pub(crate) fn bundle_identifier(exe: &std::path::Path) -> Option<String> {
    let bundle = exe
        .ancestors()
        .skip(1)
        .find(|dir| dir.extension().is_some_and(|ext| ext == "app"))?;
    let plist = std::fs::read_to_string(bundle.join("Contents/Info.plist")).ok()?;
    identifier_in_plist(&plist)
}

/// The `CFBundleIdentifier` string of an XML property list.
#[cfg(any(target_os = "macos", test))]
fn identifier_in_plist(plist: &str) -> Option<String> {
    let after = plist.split_once("<key>CFBundleIdentifier</key>")?.1;
    let value = after.split_once("<string>")?.1.split_once("</string>")?.0;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_identifier_from_an_xml_plist() {
        let plist = "<?xml version=\"1.0\"?><plist><dict>\n<key>CFBundleName</key><string>Oxikube</string>\n<key>CFBundleIdentifier</key>\n  <string>dev.oxikube.app</string>\n</dict></plist>";
        assert_eq!(
            identifier_in_plist(plist),
            Some("dev.oxikube.app".to_owned())
        );
        assert_eq!(identifier_in_plist("<dict></dict>"), None);
        assert_eq!(
            identifier_in_plist("<key>CFBundleIdentifier</key><string> </string>"),
            None
        );
    }

    #[test]
    fn an_unbundled_binary_has_no_identifier() {
        use std::path::Path;
        assert_eq!(
            bundle_identifier(Path::new("/work/target/debug/oxikube")),
            None
        );
        // A path inside a bundle that does not exist on disk: nothing to read.
        assert_eq!(
            bundle_identifier(Path::new("/nonexistent/Oxikube.app/Contents/MacOS/oxikube")),
            None
        );
    }
}
