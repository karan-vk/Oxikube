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
//! container (E09-S10).
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
//! Users rebind them in `keymap.json`.

use gpui::actions;

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
        /// Add a debug container to the cursor row's pod (`pod::Debug`, through the debug dialog;
        /// `shift-d`). A toast says why not for a kind that is not a pod, or on a read-only cluster.
        DebugSelected,
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
        /// Show or hide the wide columns of this kind's tables (`table::ToggleWide`; k9s's
        /// `ctrl-w`).
        ToggleWide,
    ]
);
