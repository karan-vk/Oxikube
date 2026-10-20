//! Validation: port ranges, Auto against fixed, the default name, the bind warning.

use oxikube_domain::portforward::ForwardPort;

use super::target;
use crate::dialog::{FormInput, PortChoice, StartForm};

fn form() -> StartForm {
    StartForm::new(
        target("Pod", "web"),
        vec![PortChoice {
            number: 8080,
            name: Some("http".into()),
            container: Some("app".into()),
        }],
    )
}

fn input(remote: &str) -> FormInput {
    FormInput {
        remote_port: remote.into(),
        ..FormInput::default()
    }
}

#[test]
fn auto_is_the_default_and_builds_local_port_zero_on_loopback() {
    let spec = form().build(&input("8080")).expect("valid");
    assert_eq!(spec.local_port, 0);
    assert_eq!(spec.remote_port, ForwardPort::Number(8080));
    assert!(spec.bind.is_loopback());
    assert_eq!(spec.name, None);
    assert!(!spec.favourite);
}

#[test]
fn a_fixed_local_port_is_checked_against_1_to_65535() {
    let form = form();
    let fixed = |text: &str| FormInput {
        local_auto: false,
        local_port: text.into(),
        ..input("8080")
    };
    assert_eq!(form.build(&fixed("9090")).unwrap().local_port, 9090);
    for bad in ["0", "65536", "-1", "abc", ""] {
        let errors = form.build(&fixed(bad)).unwrap_err();
        assert!(errors.local_port.is_some(), "{bad:?} should be refused");
    }
}

#[test]
fn the_configured_range_limits_the_local_port() {
    let form = form().with_range(10000..=10100);
    let fixed = FormInput {
        local_auto: false,
        local_port: "9090".into(),
        ..input("8080")
    };
    let errors = form.build(&fixed).unwrap_err();
    assert!(errors.local_port.unwrap().contains("10000"));
    let inside = FormInput {
        local_port: "10050".into(),
        ..fixed
    };
    assert!(form.build(&inside).is_ok());
    // Auto ignores the range: the OS picks.
    assert!(form.build(&input("8080")).is_ok());
}

#[test]
fn the_remote_port_is_a_number_or_a_name_the_target_has() {
    let form = form();
    assert_eq!(
        form.build(&input("http")).unwrap().remote_port,
        ForwardPort::Number(8080)
    );
    assert!(
        form.build(&input("nope"))
            .unwrap_err()
            .remote_port
            .is_some()
    );
    assert!(form.build(&input("0")).unwrap_err().remote_port.is_some());
    assert!(form.build(&input("")).unwrap_err().remote_port.is_some());
    // An untouched field is not shouted at while typing, but Forward stays off.
    assert!(form.errors(&input("")).remote_port.is_none());
    assert!(!form.can_submit(&input("")));
}

#[test]
fn the_default_name_is_kind_name_port_and_a_typed_name_wins() {
    let form = form();
    assert_eq!(form.default_name("8080"), "pod/web:8080");
    let service = StartForm::new(target("Service", "api"), Vec::new());
    assert_eq!(service.default_name("80"), "service/api:80");
    let named = FormInput {
        name: "  my api ".into(),
        favourite: true,
        ..input("8080")
    };
    let spec = form.build(&named).unwrap();
    assert_eq!(spec.name.as_deref(), Some("my api"));
    assert!(spec.favourite);
    assert_eq!(spec.default_name(), "pod/web:8080");
}

#[test]
fn a_non_loopback_bind_warns_but_does_not_block() {
    let form = form();
    let open = FormInput {
        bind: "0.0.0.0".into(),
        ..input("8080")
    };
    assert!(form.bind_warning(&open).unwrap().contains("other machines"));
    assert!(form.build(&open).is_ok());
    assert_eq!(form.bind_warning(&input("8080")), None);
    let v6 = FormInput {
        bind: "::1".into(),
        ..input("8080")
    };
    assert_eq!(form.bind_warning(&v6), None);
    let bad = FormInput {
        bind: "localhost".into(),
        ..input("8080")
    };
    assert!(form.build(&bad).unwrap_err().bind.is_some());
}

#[test]
fn a_privileged_local_port_is_hinted() {
    let form = form();
    let low = FormInput {
        local_auto: false,
        local_port: "80".into(),
        ..input("8080")
    };
    assert!(form.local_warning(&low).unwrap().contains("1024"));
    let high = FormInput {
        local_port: "8080".into(),
        ..low
    };
    assert_eq!(form.local_warning(&high), None);
}
