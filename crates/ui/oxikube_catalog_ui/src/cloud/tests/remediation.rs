//! The remediation table: every issue has words for every provider, the commands are the ones
//! the CLIs document, and errors map onto issues by kind.

use oxikube_domain::OxiError;
use oxikube_domain::cloud::{CloudProvider, CloudScope};

use crate::cloud::{Issue, remediation};

fn aws_scope() -> CloudScope {
    CloudScope::Aws {
        profile: "prod".into(),
        region: "eu-west-1".into(),
    }
}

#[test]
fn every_issue_has_words_for_every_provider() {
    for provider in CloudProvider::ALL {
        for issue in Issue::ALL {
            let fix = remediation(provider, issue, None);
            assert_eq!(fix.issue, issue);
            assert!(
                fix.text.ends_with('.'),
                "{provider} {issue:?}: {}",
                fix.text
            );
            assert!(
                !fix.text.contains("exit "),
                "no raw CLI failure: {}",
                fix.text
            );
        }
    }
}

#[test]
fn login_commands_are_the_documented_ones() {
    let command = |provider, scope| remediation(provider, Issue::NotLoggedIn, scope).command;
    let prod = aws_scope();
    assert_eq!(
        command(CloudProvider::Aws, Some(&prod)).as_deref(),
        Some("aws sso login --profile prod")
    );
    assert_eq!(
        command(CloudProvider::Aws, None).as_deref(),
        Some("aws sso login --profile <profile>")
    );
    assert_eq!(
        command(CloudProvider::Gcp, None).as_deref(),
        Some("gcloud auth login")
    );
    assert_eq!(
        command(CloudProvider::Azure, None).as_deref(),
        Some("az login")
    );
}

#[test]
fn plugin_commands_name_the_plugin() {
    let gcp = remediation(CloudProvider::Gcp, Issue::PluginMissing, None);
    assert_eq!(
        gcp.command.as_deref(),
        Some("gcloud components install gke-gcloud-auth-plugin")
    );
    let azure = remediation(CloudProvider::Azure, Issue::PluginMissing, None);
    assert!(azure.text.contains("kubelogin"));
    assert_eq!(azure.command.as_deref(), Some("az aks install-cli"));
}

#[test]
fn errors_map_to_issues_by_kind_and_wording() {
    let of = |error: OxiError| Issue::of(&error);
    assert_eq!(of(OxiError::auth("x", false)), Issue::NotLoggedIn);
    assert_eq!(of(OxiError::forbidden("x")), Issue::Forbidden);
    assert_eq!(of(OxiError::network("x")), Issue::Network);
    assert_eq!(of(OxiError::timeout("x")), Issue::Timeout);
    assert_eq!(of(OxiError::internal("x")), Issue::Other);
    assert_eq!(
        of(OxiError::unsupported("`aws` was not found on the PATH")),
        Issue::CliMissing
    );
    assert_eq!(
        of(OxiError::unsupported(
            "this version of `az` does not support it; upgrade it"
        )),
        Issue::CliTooOld
    );
    assert_eq!(
        of(OxiError::unsupported(
            "gke-gcloud-auth-plugin is not installed (the auth plugin)"
        )),
        Issue::PluginMissing
    );
}

#[test]
fn unsupported_errors_keep_their_own_message_unless_the_cli_is_missing() {
    let kinds = [
        "the Kubernetes Engine API is not enabled in project p: run `gcloud services enable x`",
        "AKS cluster `x` is stopped; start it with `az aks start -n x`",
        "AKS cluster `x` signs in with local accounts only. Enable Azure AD integration",
        "`aws --version` printed something unexpected: hmm",
    ];
    for message in kinds {
        assert_eq!(
            Issue::of(&OxiError::unsupported(message)),
            Issue::Other,
            "{message}"
        );
    }
    let missing = OxiError::unsupported("`gcloud` was not found on the login shell's PATH");
    assert_eq!(Issue::of(&missing), Issue::CliMissing);
    let plugin = OxiError::unsupported("`kubelogin` is not installed, so cluster cannot connect");
    assert_eq!(Issue::of(&plugin), Issue::PluginMissing);
    let old = OxiError::unsupported("this version of `az` does not support `x`; upgrade it");
    assert_eq!(Issue::of(&old), Issue::CliTooOld);
}

#[test]
fn missing_regions_has_its_own_issue() {
    let error = OxiError::validation("no AWS regions to scan: add regions");
    assert_eq!(Issue::of(&error), Issue::NoRegions);
}
