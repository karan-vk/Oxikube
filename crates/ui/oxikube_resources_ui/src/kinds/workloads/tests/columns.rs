//! The workload tables show the columns of the story with the cells of the domain view-models.

use gpui::TestAppContext;
use oxikube_app::ColumnId;

use super::{deployments, web_deployment};
use crate::table::tests::fixture::{Fixture, cluster};

#[gpui::test]
fn the_deployments_table_shows_ready_up_to_date_available_replicas_age_and_conditions(
    cx: &mut TestAppContext,
) {
    let mut f = Fixture::new(cx);
    f.vcx.update(|_, cx| crate::kinds::init(cx));
    let ports = f.ports();
    ports.discovery.set_kinds([deployments()]);
    ports.resources.insert(web_deployment());
    futures::executor::block_on(f.sessions.connect(&cluster())).expect("connect");
    f.vcx.run_until_parked();
    let table = f.open(deployments());
    let columns = f
        .vcx
        .update(|_, cx| table.read(cx).read_rows(cx, |d| d.layout().visible_ids()));
    let shown: Vec<&str> = columns.iter().map(ColumnId::as_str).collect();
    for wanted in [
        "name",
        "ready",
        "up-to-date",
        "available",
        "replicas",
        "age",
        "conditions",
    ] {
        assert!(shown.contains(&wanted), "{wanted} in {shown:?}");
    }
    assert_eq!(f.names(&table), ["web"]);
}
