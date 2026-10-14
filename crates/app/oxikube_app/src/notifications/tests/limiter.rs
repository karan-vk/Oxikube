//! The rate limit: per row and overall; a burst of 1 000 raises a handful.

use super::{at, cluster, grouped, input};
use crate::notifications::NotificationRules;
use crate::notifications::centre::Centre;
use crate::notifications::limiter::{Limiter, Route};

#[test]
fn at_most_max_raised_per_window_whatever_the_rows() {
    let rules = NotificationRules::default();
    let mut limiter = Limiter::default();
    let raised = (0..20)
        .filter(|_| limiter.may_raise(&rules, None, false, at(0)))
        .count();
    assert_eq!(raised, rules.max_raised);
    let window = rules.raise_window.as_secs() as i64;
    assert!(
        limiter.may_raise(&rules, None, false, at(window)),
        "the window moved on"
    );
}

#[test]
fn routing_counts_distinct_objects_in_the_window() {
    let rules = NotificationRules::default();
    let mut limiter = Limiter::default();
    let reason = "BackOff".into();
    let routes: Vec<Route> = (0..5)
        .map(|i| limiter.route(&rules, Some(&cluster()), &reason, at(i)))
        .collect();
    assert_eq!(
        routes,
        [
            Route::Own,
            Route::Own,
            Route::Own,
            Route::Group,
            Route::Group
        ]
    );
    limiter.forget(&cluster());
    assert_eq!(
        limiter.route(&rules, Some(&cluster()), &reason, at(6)),
        Route::Own
    );
}

#[test]
fn a_burst_of_a_thousand_warnings_in_a_second_raises_a_handful() {
    let rules = NotificationRules::default();
    let mut centre = Centre::default();
    let mut limiter = Limiter::default();
    let mut raised = 0;
    for i in 0..1_000 {
        let reason = if i % 2 == 0 { "BackOff" } else { "Unhealthy" };
        let applied = centre.apply(
            grouped(&cluster(), &format!("pod-{i}"), reason),
            &mut limiter,
            &rules,
            at(0),
        );
        raised += usize::from(applied.raised);
    }
    assert!(raised <= rules.max_raised, "{raised} toasts");
    assert!(raised >= 2, "the burst is still told");
    // Two reasons: three rows of their own and one group each.
    assert_eq!(centre.len(), 8);
    let groups: Vec<_> = centre.list().into_iter().filter(|n| n.grouped).collect();
    assert_eq!(groups.len(), 2);
    assert!(groups.iter().all(|g| g.objects == 497), "{groups:?}");
    // Plain inputs of many different keys are limited the same way.
    let mut limiter = Limiter::default();
    let mut centre = Centre::default();
    let raised = (0..1_000)
        .filter(|i| {
            centre
                .apply(
                    input(&cluster(), &format!("k{i}")),
                    &mut limiter,
                    &rules,
                    at(0),
                )
                .raised
        })
        .count();
    assert_eq!(raised, rules.max_raised);
}
