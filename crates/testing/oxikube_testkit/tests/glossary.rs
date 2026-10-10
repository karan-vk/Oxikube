//! Keeps `docs/CONTEXT.md` honest: every `oxikube_domain::<module>`,
//! `oxikube_ports::<module>` and `oxikube_testkit::<module>` path in the glossary's "Lives in"
//! column must name a module that exists, and a second test fails to compile when a
//! glossary type is renamed or removed.

use std::path::{Path, PathBuf};

const CONTEXT: &str = include_str!("../../../../docs/CONTEXT.md");

/// Source directory of a crate under `crates/`, or `None` for crates we do not check.
fn src_dir(krate: &str) -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let layer = match krate {
        "oxikube_domain" => "domain",
        "oxikube_ports" => "ports",
        "oxikube_testkit" => "testing",
        _ => return None,
    };
    Some(root.join(layer).join(krate).join("src"))
}

fn module_exists(src: &Path, module: &str) -> bool {
    src.join(format!("{module}.rs")).is_file() || src.join(module).is_dir()
}

/// The modules named by one code span such as `oxikube_ports::{discovery, log}` or
/// `oxikube_domain::agent::thread` (only the first path segment is checked: deeper segments
/// may be marked planned).
fn modules_in(span: &str) -> Option<(&str, Vec<String>)> {
    let (krate, rest) = span.split_once("::")?;
    src_dir(krate)?;
    let krate = ["oxikube_domain", "oxikube_ports", "oxikube_testkit"]
        .into_iter()
        .find(|k| *k == krate)?;
    let modules = match rest.strip_prefix('{') {
        Some(list) => list
            .trim_end_matches('}')
            .split(',')
            .map(|m| m.trim().split("::").next().unwrap_or_default().to_owned())
            .collect(),
        None => vec![rest.split("::").next().unwrap_or_default().to_owned()],
    };
    Some((krate, modules))
}

#[test]
fn glossary_module_paths_exist() {
    let mut checked = 0;
    for line in CONTEXT.lines().filter(|l| l.starts_with('|')) {
        let Some(lives_in) = line.trim_end_matches('|').rsplit('|').next() else {
            continue;
        };
        // Pieces alternate text / code span. A span preceded by `(planned)` is not checked.
        for (text, span) in lives_in
            .split('`')
            .zip(lives_in.split('`').skip(1))
            .step_by(2)
        {
            if text.contains("(planned)") {
                continue;
            }
            let Some((krate, modules)) = modules_in(span) else {
                continue;
            };
            let src = src_dir(krate).expect("checked in modules_in");
            for module in modules {
                assert!(
                    module_exists(&src, &module),
                    "docs/CONTEXT.md names `{krate}::{module}` but {} has no such module; \
                     fix the glossary or mark the path planned",
                    src.display()
                );
                checked += 1;
            }
        }
    }
    assert!(
        checked >= 30,
        "parsed only {checked} module paths; did the table format change?"
    );
}

/// Every type the glossary names as living in the domain or ports must keep its name and path.
#[test]
fn glossary_types_exist() {
    fn name<T: ?Sized>() -> &'static str {
        std::any::type_name::<T>()
    }
    let _watch_feed: Option<oxikube_ports::feed::WatchFeed> = None;
    let _table_feed: Option<oxikube_ports::table::TableFeed> = None;
    let names = [
        name::<oxikube_domain::ids::ClusterId>(),
        name::<oxikube_domain::ids::ContextName>(),
        name::<oxikube_domain::ids::Gvk>(),
        name::<oxikube_domain::ids::Gvr>(),
        name::<oxikube_domain::ids::Scope>(),
        name::<oxikube_domain::ids::ResourceRef>(),
        name::<oxikube_domain::kinds::ResourceKind>(),
        name::<oxikube_domain::kinds::Verb>(),
        name::<oxikube_domain::kinds::VerbSet>(),
        name::<oxikube_domain::resource::Resource>(),
        name::<oxikube_domain::resource::ObjectMeta>(),
        name::<oxikube_domain::resource::OwnerRef>(),
        name::<oxikube_domain::view::PodSummary>(),
        name::<oxikube_domain::view::ContainerSummary>(),
        name::<oxikube_domain::view::NodeSummary>(),
        name::<oxikube_domain::view::WorkloadSummary>(),
        name::<oxikube_domain::view::JobSummary>(),
        name::<oxikube_domain::view::CronJobSummary>(),
        name::<oxikube_domain::quantity::Quantity>(),
        name::<oxikube_domain::schema::JsonSchema>(),
        name::<oxikube_domain::schema::SchemaType>(),
        name::<oxikube_domain::age::Age>(),
        name::<oxikube_domain::age::AgeStyle>(),
        name::<oxikube_domain::log::LogLine>(),
        name::<oxikube_domain::event::Event>(),
        name::<oxikube_domain::event::EventType>(),
        name::<oxikube_domain::metrics::MetricsSample>(),
        name::<oxikube_domain::metrics::MetricsSubject>(),
        name::<oxikube_domain::metrics::Reading>(),
        name::<oxikube_domain::metrics::MissingReason>(),
        name::<oxikube_domain::error::OxiError>(),
        name::<oxikube_domain::error::ErrorKind>(),
        name::<oxikube_domain::session::ClusterSessionState>(),
        name::<oxikube_domain::session::SessionEvent>(),
        name::<oxikube_domain::session::SessionPhase>(),
        name::<oxikube_domain::session::InvalidTransition>(),
        name::<oxikube_domain::session::NamespaceSelection>(),
        name::<oxikube_domain::session::NamespaceFavourites>(),
        name::<oxikube_domain::session::WatchScope>(),
        name::<oxikube_domain::command::Command>(),
        name::<oxikube_domain::command::CommandId>(),
        name::<oxikube_domain::command::CommandMeta>(),
        name::<oxikube_domain::command::CommandScope>(),
        name::<oxikube_domain::command::Capability>(),
        name::<oxikube_domain::command::Capabilities>(),
        name::<oxikube_domain::safety::Risk>(),
        name::<oxikube_domain::safety::ConfirmTier>(),
        name::<oxikube_domain::safety::Initiator>(),
        name::<oxikube_domain::audit::AuditRecord>(),
        name::<oxikube_domain::audit::AuditOutcome>(),
        name::<oxikube_domain::agent::ContextBlock>(),
        name::<oxikube_ports::feed::Delta>(),
        name::<oxikube_ports::feed::DeltaBatch>(),
        name::<oxikube_ports::table::Table>(),
        name::<dyn oxikube_ports::resource::ResourceReader>(),
        name::<dyn oxikube_ports::resource::ResourceWriter>(),
        name::<dyn oxikube_ports::resource::ResourcePort>(),
        name::<oxikube_ports::integration::SidebarModel>(),
        name::<oxikube_ports::tool::ToolDef>(),
        name::<oxikube_ports::tool::ToolName>(),
        name::<oxikube_ports::tool::ToolAnnotations>(),
        name::<oxikube_ports::context::Mention>(),
        name::<oxikube_ports::context::MentionPrefix>(),
        name::<oxikube_ports::context::ContentPart>(),
        name::<oxikube_ports::agent::AgentSessionId>(),
        name::<oxikube_ports::agent::SessionUpdate>(),
        name::<dyn oxikube_ports::integration::IntegrationPort>(),
        name::<dyn oxikube_ports::tool::ToolPort>(),
        name::<dyn oxikube_ports::context::ContextProviderPort>(),
        name::<dyn oxikube_ports::agent::AgentPort>(),
        name::<dyn oxikube_ports::agent::AgentClient>(),
        name::<dyn oxikube_ports::schema::SchemaPort>(),
    ];
    // Every name that appears in the table as a bold term or a code span must be a real type.
    for n in names {
        let short = n
            .rsplit("::")
            .next()
            .unwrap_or(n)
            .trim_start_matches("dyn ");
        let short = short.split('<').next().unwrap_or(short);
        assert!(
            CONTEXT.contains(short),
            "type `{n}` exists but docs/CONTEXT.md never mentions `{short}`"
        );
    }
}
