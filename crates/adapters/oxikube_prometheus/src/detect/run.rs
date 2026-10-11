//! One detection run: list, match, probe, verdict.

use std::collections::BTreeMap;

use futures::{StreamExt as _, stream};
use jiff::Timestamp;
use oxikube_domain::MetricsProviderKind;
use oxikube_ports::{
    DetectedProvider, DetectionFailure, DetectionResult, DetectionState, Probe, ProbeOutcome,
    PromEndpoint, PromqlEndpointPort, ProviderTarget, ResourceReader, ServiceRef,
};

use super::ProviderDetector;
use super::catalogue::Entry;
use super::services::{Listing, ServiceInfo, list};
use crate::errors::clean;

/// Most namespace lists in flight at once.
const CONCURRENT_LISTS: usize = 4;

/// The first failed probe, kept for the verdict.
struct FailedProbe {
    kind: MetricsProviderKind,
    service: ServiceRef,
    reason: String,
}

impl ProviderDetector {
    /// Looks for a provider in the cluster `resources` reads, probing through `client` (which
    /// routes Service endpoints through that cluster's service proxy).
    pub(super) async fn run(
        &self,
        resources: &dyn ResourceReader,
        client: &dyn PromqlEndpointPort,
    ) -> DetectionResult {
        let namespaces: Vec<String> = self
            .catalogue
            .namespaces()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let listed: Vec<(String, Listing)> = stream::iter(namespaces)
            .map(|namespace: String| async move {
                let listing = list(resources, &namespace).await;
                (namespace, listing)
            })
            .buffered(CONCURRENT_LISTS)
            .collect()
            .await;
        let listings: BTreeMap<String, Listing> = listed.into_iter().collect();

        let mut probes = Vec::new();
        let mut found: Option<DetectedProvider> = None;
        let mut failure: Option<FailedProbe> = None;

        for entry in &self.catalogue.entries {
            for namespace in &entry.namespaces {
                let record = |service: String, outcome| Probe {
                    namespace: namespace.clone(),
                    service,
                    kind: entry.kind,
                    outcome,
                };
                let matched: Vec<&ServiceInfo> = match listings.get(namespace.as_str()) {
                    Some(Listing::Services(all)) => {
                        all.iter().filter(|s| entry.matches(&s.name)).collect()
                    }
                    Some(Listing::Failed { reason, .. }) => {
                        probes.push(record(
                            entry.pattern(),
                            ProbeOutcome::Skipped(reason.clone()),
                        ));
                        continue;
                    }
                    None => continue,
                };
                if matched.is_empty() {
                    probes.push(record(entry.pattern(), ProbeOutcome::NotPresent));
                }
                for service in matched {
                    if found.is_some() {
                        probes.push(record(service.name.clone(), ProbeOutcome::NotNeeded));
                        continue;
                    }
                    let Some(port) = service.pick_port(&entry.ports) else {
                        probes.push(record(service.name.clone(), ProbeOutcome::NoUsablePort));
                        continue;
                    };
                    let target = ServiceRef::new(namespace.clone(), service.name.clone(), port)
                        .with_scheme(entry.scheme.into());
                    match self.probe(entry, &target, client).await {
                        Ok(path_prefix) => {
                            probes.push(record(service.name.clone(), ProbeOutcome::Answered));
                            found = Some(DetectedProvider {
                                kind: entry.kind,
                                target: ProviderTarget::Service(target),
                                path_prefix,
                            });
                        }
                        Err(reason) => {
                            probes.push(record(
                                service.name.clone(),
                                ProbeOutcome::Failed(reason.clone()),
                            ));
                            failure.get_or_insert(FailedProbe {
                                kind: entry.kind,
                                service: target,
                                reason,
                            });
                        }
                    }
                }
            }
        }

        let state = match (found, failure) {
            (Some(provider), _) => DetectionState::Found(provider),
            (None, Some(failed)) => DetectionState::Failed(DetectionFailure::ProbeFailed {
                kind: failed.kind,
                target: ProviderTarget::Service(failed.service),
                reason: failed.reason,
            }),
            (None, None) => verdict_without_candidates(&listings),
        };
        DetectionResult {
            state,
            probes,
            finished_at: Some(Timestamp::now()),
        }
    }

    /// Sends the probe query to `target` under each of the entry's path prefixes; the prefix that
    /// answers, or the first failure's reason.
    async fn probe(
        &self,
        entry: &Entry,
        target: &ServiceRef,
        client: &dyn PromqlEndpointPort,
    ) -> Result<Option<String>, String> {
        let mut first_error = None;
        for prefix in entry.prefixes() {
            let mut endpoint =
                PromEndpoint::service(target.clone()).with_timeout(self.probe_timeout);
            endpoint.path_prefix = prefix.clone();
            match client.ping(&endpoint).await {
                Ok(()) => return Ok(prefix),
                Err(error) => {
                    // A missing endpoint or a refused proxy fails the same under every prefix.
                    let stop = error.is_absent()
                        || matches!(
                            error,
                            oxikube_ports::PromqlError::Forbidden(_)
                                | oxikube_ports::PromqlError::Unauthorized(_)
                        );
                    first_error.get_or_insert_with(|| clean(&error.to_string()));
                    if stop {
                        break;
                    }
                }
            }
        }
        Err(first_error.unwrap_or_else(|| "no answer".to_owned()))
    }
}

/// Nothing matched and nothing failed to probe: absence if every namespace was read, else a
/// failure that says where the blind spots are.
fn verdict_without_candidates(listings: &BTreeMap<String, Listing>) -> DetectionState {
    let mut forbidden = Vec::new();
    let mut other = Vec::new();
    for (namespace, listing) in listings {
        if let Listing::Failed {
            forbidden: f,
            reason,
        } = listing
        {
            if *f {
                forbidden.push(namespace.as_str());
            } else {
                other.push(format!("{namespace}: {reason}"));
            }
        }
    }
    if !forbidden.is_empty() {
        return DetectionState::Failed(DetectionFailure::Forbidden(format!(
            "cannot list services in {}",
            forbidden.join(", ")
        )));
    }
    if !other.is_empty() {
        return DetectionState::Failed(DetectionFailure::Other(format!(
            "could not look for a metrics service ({})",
            other.join("; ")
        )));
    }
    DetectionState::NotFound
}
