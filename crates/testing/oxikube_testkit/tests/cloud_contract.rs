//! `FakeCloudDiscoveryPort` satisfies the same `CloudDiscoveryPort` contract the `oxikube_cloud`
//! adapters are held to (E18-S01), for every provider and for a missing CLI.

use futures::executor::block_on;
use oxikube_domain::OxiError;
use oxikube_ports::{
    CloudDetection, CloudProvider, CloudScope, CloudToolStatus, ClusterStatus, DiscoveredCluster,
    SetupHint,
};
use oxikube_testkit::FakeCloudDiscoveryPort;
use oxikube_testkit::contracts::{CloudContractReport, cloud_discovery};

fn scopes(provider: CloudProvider) -> Vec<CloudScope> {
    match provider {
        CloudProvider::Aws => vec![
            CloudScope::Aws {
                profile: "prod".into(),
                region: "eu-west-1".into(),
            },
            CloudScope::Aws {
                profile: "prod".into(),
                region: "us-east-1".into(),
            },
        ],
        CloudProvider::Gcp => vec![CloudScope::Gcp {
            project: "p-1".into(),
        }],
        CloudProvider::Azure => vec![CloudScope::Azure {
            subscription_id: "00000000-0000-0000-0000-000000000001".into(),
            subscription_name: Some("Dev".into()),
        }],
    }
}

fn clusters(scopes: &[CloudScope]) -> Vec<DiscoveredCluster> {
    let mut out = Vec::new();
    for scope in scopes {
        out.push(DiscoveredCluster::new(scope.clone(), "alpha", "zone-a"));
        out.push(DiscoveredCluster {
            endpoint: Some("https://203.0.113.10".into()),
            ca_data: Some("LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0t".into()),
            status: ClusterStatus::Updating,
            ..DiscoveredCluster::new(scope.clone(), "beta", "zone-b")
        });
    }
    out
}

#[test]
fn a_ready_fake_meets_the_contract_for_every_provider() {
    for provider in CloudProvider::ALL {
        let scopes = scopes(provider);
        let fake = FakeCloudDiscoveryPort::ready(provider)
            .with_scopes(scopes.clone())
            .with_clusters(clusters(&scopes));
        let report = block_on(cloud_discovery(&fake));
        assert_eq!(
            report,
            CloudContractReport {
                scopes: scopes.len(),
                clusters: scopes.len() * 2,
            },
            "{provider}"
        );
    }
}

#[test]
fn a_missing_cli_meets_the_contract_with_nothing_listed() {
    for provider in CloudProvider::ALL {
        let fake = FakeCloudDiscoveryPort::new(provider);
        assert_eq!(
            block_on(cloud_discovery(&fake)),
            CloudContractReport::default()
        );
        let unsupported = FakeCloudDiscoveryPort::new(provider);
        unsupported
            .script()
            .scopes
            .push_err(OxiError::unsupported("no CLI"));
        assert_eq!(
            block_on(cloud_discovery(&unsupported)),
            CloudContractReport::default()
        );
    }
}

#[test]
fn a_logged_out_cli_still_reports_its_binary_and_a_hint() {
    let detection = CloudDetection {
        provider: CloudProvider::Azure,
        status: CloudToolStatus::NotAuthenticated,
        binary: Some("/opt/homebrew/bin/az".into()),
        version: Some("2.64.0".into()),
        hints: vec![SetupHint::with_command("az is not logged in", "az login")],
    };
    let fake = FakeCloudDiscoveryPort::new(CloudProvider::Azure).with_detection(detection);
    assert_eq!(block_on(cloud_discovery(&fake)).scopes, 0);
}

#[test]
#[should_panic(expected = "carries another scope")]
fn the_contract_catches_a_cluster_listed_under_the_wrong_scope() {
    let scopes = scopes(CloudProvider::Aws);
    let wrong = DiscoveredCluster::new(scopes[1].clone(), "alpha", "zone-a");
    let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Aws).with_scopes(scopes);
    // The contract lists a foreign scope first, then each scope in order.
    fake.script()
        .list_clusters
        .push_err(OxiError::validation("foreign scope"))
        .push_ok(vec![wrong]);
    block_on(cloud_discovery(&fake));
}

#[test]
#[should_panic(expected = "secret-looking")]
fn the_contract_catches_a_token_in_an_entry() {
    let scopes = scopes(CloudProvider::Gcp);
    let cluster = DiscoveredCluster::new(scopes[0].clone(), "alpha", "zone-a");
    let fake = FakeCloudDiscoveryPort::ready(CloudProvider::Gcp)
        .with_scopes(scopes)
        .with_clusters([cluster.clone()]);
    let mut entry = block_on(oxikube_ports::CloudDiscoveryPort::kubeconfig_entry(
        &fake, &cluster,
    ))
    .expect("the fake builds an entry");
    entry
        .exec
        .args
        .push("--token=eyJhbGciOiJSUzI1NiIsImtpZCI6IjEifQ.eyJzdWIiOiJ4In0.c2lnbmF0dXJl".into());
    fake.script().kubeconfig_entry.push_ok(entry);
    block_on(cloud_discovery(&fake));
}
