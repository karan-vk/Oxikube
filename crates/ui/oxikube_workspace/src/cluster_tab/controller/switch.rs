//! The tab order and switching: `cmd-1..9`, next and previous, `cluster::Select`.

use gpui::{App, Context, Window};
use oxikube_domain::ids::ClusterId;

use super::{ClusterTabs, ClusterTabsEvent};

impl ClusterTabs {
    /// The open clusters in the order their tabs are shown: the window's panes in layout order,
    /// each pane's tabs left to right. Dragging a tab changes it. Clusters whose tab has not
    /// reached a pane yet come last, in open order.
    pub(super) fn display_order(&self, cx: &App) -> Vec<ClusterId> {
        let mut order = Vec::with_capacity(self.tabs.len());
        if let Some(workspace) = self.workspace.upgrade() {
            let workspace = workspace.read(cx);
            for pane in workspace.panes(cx) {
                for item in pane.items() {
                    if let Some((cluster, _)) =
                        self.tabs.iter().find(|(_, entry)| entry.item == *item)
                    {
                        order.push(cluster.clone());
                    }
                }
            }
        }
        for cluster in self.tabs.keys() {
            if !order.contains(cluster) {
                order.push(cluster.clone());
            }
        }
        order
    }

    /// Shows `cluster`'s tab and focuses it. Returns whether the cluster has a tab.
    pub fn activate(
        &mut self,
        cluster: &ClusterId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(entry) = self.tabs.get(cluster) else {
            return false;
        };
        let item = entry.item;
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |ws, cx| ws.activate_item(item, true, window, cx));
        }
        true
    }

    /// Shows the `index`th tab (`0` is the first), as `cmd-1` does. Returns whether there is one.
    pub fn switch_to_index(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.display_order(cx).get(index).cloned() {
            Some(cluster) => self.activate(&cluster, window, cx),
            None => false,
        }
    }

    /// Shows the tab after the displayed one, wrapping around. With no cluster displayed, the
    /// first. Returns whether there is a tab to show.
    pub fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.step(1, window, cx)
    }

    /// Shows the tab before the displayed one, wrapping around. With no cluster displayed, the
    /// last. Returns whether there is a tab to show.
    pub fn previous(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.step(-1, window, cx)
    }

    fn step(&mut self, by: isize, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let order = self.display_order(cx);
        if order.is_empty() {
            return false;
        }
        let len = order.len() as isize;
        // What the window shows now: the dock tells the tabs they are shown a turn later, so two
        // steps in one update (an immediate command, then another) must not both start from the
        // tab shown before the first.
        let shown = self.displayed(cx);
        let current = shown
            .as_ref()
            .and_then(|active| order.iter().position(|cluster| cluster == active));
        let target = match current {
            Some(current) => (current as isize + by).rem_euclid(len),
            None if by > 0 => 0,
            None => len - 1,
        } as usize;
        self.activate(&order[target].clone(), window, cx)
    }

    /// The cluster whose tab the window's workspace displays now (`None` for the catalog or
    /// another item). Without the workspace, the last tab reported shown.
    fn displayed(&self, cx: &App) -> Option<ClusterId> {
        let Some(workspace) = self.workspace.upgrade() else {
            return self.active.clone();
        };
        let item = workspace.read(cx).active_item(cx)?.item_id();
        self.tabs
            .iter()
            .find(|(_, entry)| entry.item == item)
            .map(|(cluster, _)| cluster.clone())
    }

    pub(super) fn set_active(&mut self, active: Option<ClusterId>, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active.clone();
        cx.emit(ClusterTabsEvent::ActiveChanged(active));
        self.mark_dirty(cx);
        cx.notify();
    }
}
