//! The Describe tab (E07-S06): `kubectl describe`-style text for the object, read-only.
//!
//! | File | Holds |
//! |---|---|
//! | `tab` | [`DescribeState`], [`DescribeTab`] and the view's methods: starting on first show, refresh, the result applied |
//! | `render` | the toolbar (which backend rendered it, refresh) and the body: the text in a plain-text `oxikube_ui::code_view::CodeView` (laid out off the UI thread), a spinner while it is read, an error with Retry |
//!
//! The text comes from the cluster's [`DescribePort`](oxikube_ports::DescribePort): deskribe
//! renders it natively, and `kubectl describe` is the fallback for a kind deskribe does not cover
//! (or when the `describe.backend` setting says so). The call is read-only and runs on the Tokio
//! bridge (`spawn_kube`), never on the UI thread; the task is held by the view, so closing the
//! detail (or refreshing again) cancels it, and `kubectl` is killed with it.
//!
//! It starts the first time the tab is shown, so the Overview costs no describe. A slow describe
//! shows a spinner (the previous text stays under it on a refresh); a failure shows its message
//! with a Retry button instead of an empty pane, and an unsupported kind says why.

mod render;
mod tab;

pub use tab::DescribeState;
pub(super) use tab::DescribeTab;
