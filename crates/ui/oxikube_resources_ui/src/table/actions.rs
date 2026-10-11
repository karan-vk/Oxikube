//! The resource table's key actions.
//!
//! Moving the cursor and the selection is local to the view (like the catalog's `SelectNext`);
//! the actions that stand for a command dispatch it with the table's rows as its argument:
//! `OpenSelected` sends `resource::Open`, `CopyName` sends `resource::CopyName`, `SelectAll`
//! sends `resource::SelectAll`, so the palette, the context menu, the key and an agent run one
//! behaviour (non-negotiable 4).
//!
//! The default bindings live in the per-OS keymap files of `oxikube_assets`, in the sections
//! for the `ResourceTable` key context (`oxikube_keymap::contexts::RESOURCE_TABLE`): `j` / `k` and the arrows
//! move, shift extends, `enter` opens, `escape` clears, `cmd-a` / `ctrl-a` selects all,
//! `cmd-c` / `ctrl-c` copies the name, `delete` (and k9s's `ctrl-d`) opens the delete dialog,
//! `s` opens a shell in the pod, `a` attaches to it (E09-S08) and `shift-d` adds a debug
//! container (E09-S10). On a Deployment, StatefulSet, ReplicaSet or ReplicationController `s` is
//! k9s's scale instead: the keymap binds `resource_table::ScaleSelected` for those kinds (E12-S04).
//!
//! E12-S16 adds `u` (use the namespace), `p` (pin it) for the Namespaces table and `0`-`9`
//! (All, or the nth favourite) for every table: see `namespaces`.
//!
//! `c` cordons and `u` uncordons the cursor row's node (E12-S09), bound in a section scoped to
//! `kind == Node` so `u` stays free for "used by" on the kinds that get it (#397).
//!
//! The k9s verbs of E11-S07 live next to them: `y` shows the YAML, `d` describes, `e` edits, `l`
//! opens the logs, `shift-f` forwards a port, `f` lists the forwards and `ctrl-w` toggles the wide
//! columns. Each is a view action that stands for a command of the same meaning (the table in
//! `table/verbs.rs`), so the key, the context menu, the palette and an agent run
//! one behaviour.
//!
//! The vim base keymap (`base_keymap: "vim"`, E11-S09) adds `g g` / `shift-g` (first / last row),
//! `ctrl-d` / `ctrl-u` (half a page), `d d` (the delete dialog, like `delete`) and `y y` (copy the
//! name) in the same key context, on these same actions.
//!
//! A Service's own verbs (E12-S13) are bound with a `kind == Service` section: `u` shows the pods
//! its selector picks, `o` opens a port in the browser.
//!
//! Users rebind them in `keymap.json`.

use gpui::{Action, actions};
use schemars::JsonSchema;
use serde::Deserialize;

actions!(
    resource_table,
    [
        /// Move the cursor to the next row and select it.
        SelectNext,
        /// Move the cursor to the previous row and select it.
        SelectPrevious,
        /// Move the cursor to the first row.
        SelectFirst,
        /// Move the cursor to the last row.
        SelectLast,
        /// Move the cursor one page down.
        SelectPageDown,
        /// Move the cursor one page up.
        SelectPageUp,
        /// Move the cursor half a page down (the vim base keymap's `ctrl-d`).
        SelectHalfPageDown,
        /// Move the cursor half a page up (the vim base keymap's `ctrl-u`).
        SelectHalfPageUp,
        /// Extend the selection to the next row.
        ExtendNext,
        /// Extend the selection to the previous row.
        ExtendPrevious,
        /// Open the cursor row's detail (`resource::Open`).
        OpenSelected,
        /// Copy the cursor row's name (`resource::CopyName`).
        CopyName,
        /// Select every row (`resource::SelectAll`).
        SelectAll,
        /// Clear the selection.
        ClearSelection,
        /// Delete the selected rows, or the cursor row (`resource::Delete` through the delete
        /// dialog).
        DeleteSelected,
        /// Open a shell in the cursor row's pod (`pod::Shell`; k9s's `s`). A toast says why
        /// not for a kind without a shell, or on a read-only cluster that does not allow shells; a
        /// multi-selection acts on the cursor row and says so.
        ShellSelected,
        /// Attach to the cursor row's pod (`pod::Attach`; k9s's `a`).
        AttachSelected,
        /// Scale the cursor row's workload through the scale dialog (`workload::Scale`; k9s's `s`
        /// on a Deployment, StatefulSet, ReplicaSet or ReplicationController: bound for those
        /// kinds only, where it replaces the shell key).
        ScaleSelected,
        /// Add a debug container to the cursor row's pod (`pod::Debug`, through the debug dialog;
        /// `shift-d`). A toast says why not for a kind that is not a pod, or on a read-only cluster.
        DebugSelected,
        /// Cordon the cursor row's node (`node::Cordon`; k9s's `c`, Nodes only). A toast says why
        /// not for another kind, or on a read-only cluster; a multi-selection acts on the cursor
        /// row and says so.
        CordonSelected,
        /// Uncordon the cursor row's node (`node::Uncordon`; k9s's `u` on Nodes).
        UncordonSelected,
        /// Show the cursor row's YAML in the detail drawer (`resource::ViewYaml`; k9s's `y`).
        ViewYaml,
        /// Show the cursor row's `kubectl describe` text in the detail drawer
        /// (`resource::ViewDescribe`; k9s's `d`).
        ViewDescribe,
        /// Open the cursor row's manifest in the editor (`resource::Edit`; k9s's `e`). A toast
        /// says so while no editor is installed.
        EditSelected,
        /// Open the logs of the cursor row's pod, or of the pods of its workload
        /// (`pod::ViewLogs` / `workload::ViewLogs`; k9s's `l`).
        ViewLogs,
        /// Forward a local port to the cursor row's pod (`pod::PortForward`; k9s's `shift-f`). A
        /// toast says so while port forwarding is not installed.
        PortForward,
        /// List the active port forwards (k9s's `f`). A toast says so while port forwarding is
        /// not installed.
        ShowPortForwards,
        /// Run the cursor row's CronJob now (`cronjob::Trigger`; k9s's `t`, bound on CronJob
        /// tables). Creates a Job from the template, confirmed first.
        TriggerSelected,
        /// Suspend the cursor row's CronJob or Job, or resume it when it is suspended
        /// (`cronjob::Suspend` / `cronjob::Resume`, `job::Suspend` / `job::Resume`; k9s's `s`,
        /// bound on CronJob and Job tables).
        ToggleSuspendSelected,
        /// Use the cursor row's namespace: the session watches only it (`namespace::Select` with
        /// that one name; k9s's `u`, bound for Namespaces only).
        UseNamespace,
        /// Pin or unpin the cursor row's namespace as a favourite (`namespace::ToggleFavourite`;
        /// `p`, bound for Namespaces only).
        ToggleFavouriteNamespace,
        /// Resize the cursor row's PersistentVolumeClaim (`pvc::Resize`, through the resize
        /// dialog; `shift-r`). A toast says so on any other kind, or on a read-only cluster.
        ResizeClaim,
        /// Make the cursor row's StorageClass the cluster's default (`storageclass::SetDefault`,
        /// through its dialog; `shift-s`). A toast says so on any other kind, or on a read-only
        /// cluster.
        SetDefaultClass,
        /// Show or hide the wide columns of this kind's tables (`table::ToggleWide`; k9s's
        /// `ctrl-w`).
        ToggleWide,
        /// Open the Pods table filtered by the cursor row's Service selector
        /// (`service::ShowPods`; `u`, bound where `kind == Service`).
        ShowServicePods,
        /// Forward a port of the cursor row's Service and open it in the browser
        /// (`service::OpenInBrowser`; `o`, bound where `kind == Service`). A toast says so while
        /// no port-forward manager is installed.
        OpenInBrowser,
    ]
);

/// Selects all namespaces (`slot` 0) or the nth favourite (1 to 9), as k9s's number keys do
/// (`namespace::UseFavourite`). Bound to `0`-`9` in every table, so the namespace can be changed
/// from wherever the list is.
#[derive(Clone, PartialEq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = resource_table)]
pub struct SelectNamespaceSlot {
    /// The digit.
    pub slot: u8,
}
