//! The file name the save panel suggests.

use jiff::Timestamp;

/// `<pod>-<container>-<timestamp>.log`, with the time in UTC to the second
/// (`web-0-app-20261007-120000.log`). The container is `default` while the view reads the pod's
/// default container without having learnt its name.
pub fn suggested_file_name(pod: &str, container: Option<&str>, now: Timestamp) -> String {
    format!(
        "{pod}-{}-{}.log",
        container.unwrap_or("default"),
        now.strftime("%Y%m%d-%H%M%S")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_carries_pod_container_and_time() {
        let now: Timestamp = "2026-10-07T12:00:00Z".parse().unwrap();
        assert_eq!(
            suggested_file_name("web-0", Some("app"), now),
            "web-0-app-20261007-120000.log"
        );
        assert_eq!(
            suggested_file_name("web-0", None, now),
            "web-0-default-20261007-120000.log"
        );
    }
}
