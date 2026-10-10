//! Property: what a name resolves to, and the conflict list, do not depend on the order
//! discovery listed the kinds in.

use oxikube_domain::kinds::ResourceKind;
use oxikube_testkit::kinds::{KindSpec, cert_manager_kinds, clashing_crds, core_kinds};
use proptest::prelude::*;

use crate::search::aliases::{AliasTable, Resolution};

/// Small alphabets, so groups, plurals and short names collide often.
fn arbitrary_kind() -> impl Strategy<Value = ResourceKind> {
    (
        prop::sample::select(vec!["", "a.io", "b.io", "c.io"]),
        prop::sample::select(vec!["v1", "v1beta1", "v2"]),
        prop::sample::select(vec!["Thing", "Gadget", "Widget"]),
        prop::sample::select(vec!["x", "y", "z", ""]),
        any::<bool>(),
    )
        .prop_map(|(group, version, kind, short, preferred)| {
            let mut spec = KindSpec::new(
                group,
                version,
                kind,
                &format!("{}s", kind.to_ascii_lowercase()),
            );
            if !short.is_empty() {
                spec = spec.short(short);
            }
            if !preferred {
                spec = spec.not_preferred();
            }
            spec.build()
        })
}

/// Everything observable about a table.
fn fingerprint(table: &AliasTable, probes: &[String]) -> String {
    let mut out = String::new();
    for entry in table.entries() {
        out.push_str(&format!(
            "{} -> {} ({:?})\n",
            entry.name, entry.target, entry.source
        ));
    }
    for conflict in table.conflicts() {
        out.push_str(&format!(
            "{} {:?} {} | {:?}\n",
            conflict.name,
            conflict.kind,
            conflict.winner.target,
            conflict
                .others
                .iter()
                .map(|e| e.target.to_string())
                .collect::<Vec<_>>()
        ));
    }
    for probe in probes {
        let r = table.resolve(probe);
        if let Resolution::Ambiguous(c) = &r {
            out.push_str(&format!(
                "{probe}: {:?}\n",
                c.iter().map(|e| e.target.to_string()).collect::<Vec<_>>()
            ));
        }
    }
    out
}

proptest! {
    #[test]
    fn resolution_is_independent_of_discovery_order(
        kinds in prop::collection::vec(arbitrary_kind(), 0..24),
        seed in any::<u64>(),
    ) {
        // Each (group, kind) is reported in one preferred version at most, as a server does;
        // the test still feeds non-preferred duplicates, which must lose deterministically.
        let mut shuffled = kinds.clone();
        // A cheap deterministic shuffle from the seed.
        let mut state = seed | 1;
        for i in (1..shuffled.len()).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            shuffled.swap(i, (state % (i as u64 + 1)) as usize);
        }

        let probes: Vec<String> = ["x", "y", "z", "things", "gadgets", "widgets", "thing", "dp", "po"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        let a = AliasTable::new();
        a.set_discovered(&kinds);
        let b = AliasTable::new();
        b.set_discovered(&shuffled);
        prop_assert_eq!(fingerprint(&a, &probes), fingerprint(&b, &probes));

        // Arriving one at a time as live changes reaches the same names as a full listing, when
        // each kind is delivered once in its final form.
        let mut one_each: Vec<ResourceKind> = Vec::new();
        for kind in &shuffled {
            if !one_each.iter().any(|k| k.gvk.group == kind.gvk.group && k.plural == kind.plural) {
                one_each.push(kind.clone());
            }
        }
        let c = AliasTable::new();
        c.set_discovered(&one_each);
        let d = AliasTable::new();
        for kind in &one_each {
            d.apply_kinds_change(&[], std::slice::from_ref(kind));
        }
        let names = |t: &AliasTable| t.entries().into_iter().map(|e| e.name.to_string()).collect::<Vec<_>>();
        prop_assert_eq!(names(&c), names(&d));
    }
}

#[test]
fn the_fixture_sets_resolve_the_same_in_any_rotation() {
    let mut all = core_kinds();
    all.extend(cert_manager_kinds());
    all.extend(clashing_crds());
    let reference = AliasTable::new();
    reference.set_discovered(&all);
    let probes: Vec<String> = ["certificates", "cert", "dp", "events", "ev"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let expected = fingerprint(&reference, &probes);
    for shift in 1..all.len() {
        let mut rotated = all.clone();
        rotated.rotate_left(shift);
        let table = AliasTable::new();
        table.set_discovered(&rotated);
        assert_eq!(fingerprint(&table, &probes), expected, "rotated by {shift}");
    }
}
