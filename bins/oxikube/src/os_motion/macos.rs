//! The macOS probe: `NSWorkspace` and its accessibility notification.

use block2::RcBlock;
use futures::channel::mpsc;
use objc2::{rc::Retained, runtime::ProtocolObject};
use objc2_app_kit::{
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification as DID_CHANGE,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue,
};
use std::ptr::NonNull;

use super::{OsMotionProbe, Watch};

/// Reads `accessibilityDisplayShouldReduceMotion` and observes the workspace's accessibility
/// display options notification.
pub(super) struct Workspace;

/// Removes the observer when the watch ends.
struct Observer {
    center: Retained<NSNotificationCenter>,
    token: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Drop for Observer {
    fn drop(&mut self) {
        // SAFETY: `token` is the object `addObserverForName` returned for this centre.
        unsafe { self.center.removeObserver(self.token.as_ref()) };
    }
}

impl OsMotionProbe for Workspace {
    fn watch(self: Box<Self>) -> Watch {
        let Some(main) = MainThreadMarker::new() else {
            tracing::debug!("not on the main thread: the OS reduce-motion preference is not read");
            return Watch::silent();
        };
        let (tx, rx) = mpsc::unbounded();
        let workspace = NSWorkspace::sharedWorkspace();
        // The current value first; the observer then reports each change. The notification
        // carries no payload, so the block reads the property again.
        let _ = tx.unbounded_send(workspace.accessibilityDisplayShouldReduceMotion());
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            let workspace = NSWorkspace::sharedWorkspace();
            let _ = tx.unbounded_send(workspace.accessibilityDisplayShouldReduceMotion());
        });
        let center = workspace.notificationCenter();
        // SAFETY: the block only touches a channel sender and the shared workspace, and the main
        // queue runs it on this thread, where AppKit allows both.
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(
                Some(DID_CHANGE),
                None,
                Some(&NSOperationQueue::mainQueue()),
                &block,
            )
        };
        let _ = main;
        Watch::new(rx, Observer { center, token })
    }
}
