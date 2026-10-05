use jiff::SignedDuration;

use crate::metrics::duration::parse_go_duration;

fn ms(n: i64) -> Option<SignedDuration> {
    Some(SignedDuration::from_millis(n))
}

#[test]
fn parses_the_forms_metrics_server_sends() {
    assert_eq!(parse_go_duration("14.982s"), ms(14_982));
    assert_eq!(parse_go_duration("30s"), ms(30_000));
    assert_eq!(parse_go_duration("1m0s"), ms(60_000));
    assert_eq!(parse_go_duration("1h2m3.5s"), ms(3_723_500));
    assert_eq!(parse_go_duration("250ms"), ms(250));
    assert_eq!(
        parse_go_duration("1500us"),
        Some(SignedDuration::from_micros(1500))
    );
    assert_eq!(parse_go_duration("0"), Some(SignedDuration::ZERO));
    assert_eq!(parse_go_duration("-2s"), ms(-2_000));
}

#[test]
fn rejects_everything_else() {
    for bad in ["", "-", "s", "12", "1x", "1.2.3s", "5 s", "1s garbage"] {
        assert_eq!(parse_go_duration(bad), None, "{bad:?}");
    }
}
