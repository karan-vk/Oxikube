//! Value labels: how an axis tick or a tooltip row writes a number.

/// The unit of a chart's value axis.
#[derive(Clone, Copy, Debug, Default)]
pub enum ValueFormat {
    /// A plain number, compacted past ten thousand (`12.5k`).
    #[default]
    Number,
    /// A percentage: the value is already in percent (`0..=100`).
    Percent,
    /// Bytes in binary units (`KiB`, `MiB`, `GiB`).
    Bytes,
    /// Bytes per second in binary units.
    BytesPerSecond,
    /// CPU: the value is in cores; below one core it reads in millicores (`250m`). An axis picks
    /// one unit for all its ticks, so `0, 500m, 1` never appears.
    Cores,
    /// Your own formatter: a plain function, so the format stays `Copy` and comparable.
    Custom(fn(f64) -> String),
}

impl PartialEq for ValueFormat {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ValueFormat::Custom(a), ValueFormat::Custom(b)) => std::ptr::fn_addr_eq(*a, *b),
            _ => std::mem::discriminant(self) == std::mem::discriminant(other),
        }
    }
}

const BINARY: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];

impl ValueFormat {
    /// The factor ticks are chosen in: 1 for decimal units, a power of 1024 for bytes, so the
    /// axis reads `0, 256 MiB, 512 MiB` rather than `0, 200 MB, 400 MB`.
    pub(crate) fn tick_unit(&self, max: f64) -> f64 {
        match self {
            ValueFormat::Bytes | ValueFormat::BytesPerSecond => {
                1024f64.powi(binary_exponent(max) as i32)
            }
            ValueFormat::Cores if max < 1.0 => 0.001,
            _ => 1.0,
        }
    }

    /// A value for a tooltip row (about three significant digits).
    pub fn format(&self, value: f64) -> String {
        self.write(value, None)
    }

    /// A value for an axis tick whose neighbours are `step` apart: enough decimals to tell the
    /// ticks apart, no more. `extent` is the largest value on the axis; byte labels share its unit.
    pub(crate) fn format_tick(&self, value: f64, step: f64, extent: f64) -> String {
        self.write(value, Some((step, extent)))
    }

    fn write(&self, value: f64, tick: Option<(f64, f64)>) -> String {
        let step = tick.map(|(step, _)| step);
        if !value.is_finite() {
            return "–".to_owned();
        }
        match self {
            ValueFormat::Number => number(value, step),
            ValueFormat::Percent => format!("{}%", number(value, step)),
            ValueFormat::Bytes => binary(value, tick, ""),
            ValueFormat::BytesPerSecond => binary(value, tick, "/s"),
            ValueFormat::Cores => {
                // Ticks share one unit, chosen from the axis extent (like bytes); a lone value
                // picks its own.
                let millis = match tick {
                    Some((_, extent)) => extent.abs().max(value.abs()) <= 1.0 + 1e-9,
                    None => value.abs() < 1.0 && value != 0.0,
                };
                if millis && value != 0.0 {
                    format!("{}m", number(value * 1000.0, step.map(|s| s * 1000.0)))
                } else {
                    number(value, step)
                }
            }
            ValueFormat::Custom(f) => f(value),
        }
    }
}

fn binary_exponent(max: f64) -> usize {
    let mut exponent = 0;
    let mut scaled = max.abs();
    while scaled >= 1024.0 && exponent < BINARY.len() - 1 {
        scaled /= 1024.0;
        exponent += 1;
    }
    exponent
}

fn binary(value: f64, tick: Option<(f64, f64)>, suffix: &str) -> String {
    // Tick labels share the unit of the axis' largest value; a lone value picks its own.
    let exponent =
        binary_exponent(tick.map_or(value.abs(), |(_, extent)| extent.abs().max(value.abs())));
    let scale = 1024f64.powi(exponent as i32);
    let step = tick.map(|(step, _)| step / scale);
    format!(
        "{} {}{suffix}",
        number(value / scale, step),
        BINARY[exponent]
    )
}

/// `value` with decimals suited to the `step` between ticks (or three significant digits).
fn number(value: f64, step: Option<f64>) -> String {
    let abs = value.abs();
    if step.is_none() && abs >= 10_000.0 {
        return compact(value);
    }
    let decimals = match step {
        Some(step) if step > 0.0 && step.is_finite() => decimals_of(step),
        _ => {
            if abs >= 100.0 || abs == 0.0 {
                0
            } else if abs >= 10.0 {
                1
            } else {
                2
            }
        }
    };
    trim(format!("{value:.decimals$}"))
}

/// The fewest decimals (up to four) that write `step` exactly: 0.25 needs two, 20 none.
fn decimals_of(step: f64) -> usize {
    (0..4usize)
        .find(|d| {
            let scaled = step * 10f64.powi(*d as i32);
            (scaled - scaled.round()).abs() < 1e-6 * scaled.abs().max(1.0)
        })
        .unwrap_or(4)
}

fn compact(value: f64) -> String {
    let abs = value.abs();
    let (scaled, suffix) = if abs >= 1e9 {
        (value / 1e9, "G")
    } else if abs >= 1e6 {
        (value / 1e6, "M")
    } else {
        (value / 1e3, "k")
    };
    format!("{}{suffix}", trim(format!("{scaled:.1}")))
}

/// Drops a trailing `.0` / `.50` so `2.00` reads `2` and `2.50` reads `2.5`.
fn trim(text: String) -> String {
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_keep_three_significant_digits() {
        let f = ValueFormat::Number;
        assert_eq!(f.format(0.0), "0");
        assert_eq!(f.format(1.234), "1.23");
        assert_eq!(f.format(12.34), "12.3");
        assert_eq!(f.format(123.4), "123");
        assert_eq!(f.format(12_500.0), "12.5k");
        assert_eq!(f.format(3_400_000.0), "3.4M");
        assert_eq!(f.format(f64::NAN), "–");
    }

    #[test]
    fn ticks_use_the_decimals_of_their_step() {
        let f = ValueFormat::Number;
        assert_eq!(f.format_tick(0.5, 0.25, 1.0), "0.5");
        assert_eq!(f.format_tick(0.25, 0.25, 1.0), "0.25");
        assert_eq!(f.format_tick(40.0, 20.0, 100.0), "40");
        assert_eq!(f.format_tick(1.0, 0.5, 1.0), "1");
    }

    #[test]
    fn percent_and_cores() {
        assert_eq!(ValueFormat::Percent.format(45.0), "45%");
        assert_eq!(ValueFormat::Cores.format(0.25), "250m");
        assert_eq!(ValueFormat::Cores.format(1.5), "1.5");
        assert_eq!(ValueFormat::Cores.format(0.0), "0");
        assert_eq!(ValueFormat::Cores.format_tick(0.2, 0.1, 0.4), "200m");
        // An axis reaching past a core keeps every tick in cores.
        assert_eq!(ValueFormat::Cores.format_tick(0.5, 0.5, 2.0), "0.5");
        assert_eq!(ValueFormat::Cores.format_tick(0.25, 0.25, 1.25), "0.25");
    }

    #[test]
    fn bytes_pick_a_binary_unit() {
        let f = ValueFormat::Bytes;
        assert_eq!(f.format(0.0), "0 B");
        assert_eq!(f.format(1536.0), "1.5 KiB");
        assert_eq!(f.format(256.0 * 1024.0 * 1024.0), "256 MiB");
        assert_eq!(f.format(3.5 * 1024.0 * 1024.0 * 1024.0), "3.5 GiB");
        assert_eq!(ValueFormat::BytesPerSecond.format(2048.0), "2 KiB/s");
    }

    #[test]
    fn byte_ticks_share_one_unit() {
        let f = ValueFormat::Bytes;
        let mib = 1024.0 * 1024.0;
        // 0, 256, 512 MiB: the zero tick is written in MiB too.
        assert_eq!(f.format_tick(0.0, 256.0 * mib, 768.0 * mib), "0 MiB");
        assert_eq!(
            f.format_tick(512.0 * mib, 256.0 * mib, 768.0 * mib),
            "512 MiB"
        );
        // Up to 2 GiB every label is in GiB.
        let gib = 1024.0 * mib;
        assert_eq!(f.format_tick(0.0, 0.5 * gib, 2.0 * gib), "0 GiB");
        assert_eq!(f.format_tick(0.5 * gib, 0.5 * gib, 2.0 * gib), "0.5 GiB");
    }

    #[test]
    fn custom_formatters_are_called() {
        fn plain(v: f64) -> String {
            format!("<{v}>")
        }
        assert_eq!(ValueFormat::Custom(plain).format(2.0), "<2>");
    }
}
