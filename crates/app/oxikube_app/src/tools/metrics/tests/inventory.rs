//! The registry inventory: the three tools and the provider are registered, read-only, and
//! offered to every session.

use oxikube_domain::Capabilities;
use oxikube_testkit::FakeClockPort;

use super::*;
use crate::tools::metrics::{InstallError, QUERY_PROMQL, TOP_NODES, TOP_PODS};

#[tokio::test]
async fn the_three_tools_and_the_provider_are_registered_and_read_only() {
    let h = Harness::new();
    let defs = h.tools.defs();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, [QUERY_PROMQL, TOP_NODES, TOP_PODS]);
    for def in &defs {
        assert!(!def.is_mutating(), "{}", def.name);
        assert!(
            def.risk.is_none(),
            "{} has no risk: nothing to confirm",
            def.name
        );
        assert!(def.read_only_hint(), "{}", def.name);
        def.validate().expect("a valid definition");
        assert!(!def.description.is_empty());
        assert_eq!(def.input_schema["additionalProperties"], false);
    }
    assert_eq!(
        h.tools.visible(Capabilities::empty()).len(),
        3,
        "offered to every session: absence is reported by the tool, not by hiding it"
    );
    assert_eq!(h.contexts.prefixes(), ["metrics"]);
}

#[tokio::test]
async fn installing_twice_names_the_taken_tool() {
    let h = Harness::new();
    let deps = MetricsAgentDeps {
        sessions: h.sessions.clone(),
        live: h.live.clone(),
        provider: h.provider.clone(),
        promql: h.promql.clone(),
        catalogue: h.catalogue.clone(),
        clock: Arc::new(FakeClockPort::default()),
    };
    let again = install(&h.tools, &h.contexts, &deps);
    assert!(matches!(again, Err(InstallError::Tool(_))), "{again:?}");
}
