//! [`FieldMap`]: the keys a structured line's level, time and message are looked for under.

/// The field names to look for, most specific first. The first key present in a line wins;
/// matching ignores ASCII case (`Level`, `RenderedMessage`-style loggers).
///
/// The defaults cover zap (`level`, `ts`, `msg`), logrus (`level`, `time`, `msg`), bunyan and
/// pino (`level`, `time`, `msg`) and the common aliases (`severity`, `message`, `timestamp`,
/// `@timestamp`, ...). A deployment with its own names builds a map with [`FieldMap::new`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldMap {
    /// Keys that hold the severity.
    pub level: Vec<String>,
    /// Keys that hold the time.
    pub time: Vec<String>,
    /// Keys that hold the message.
    pub message: Vec<String>,
}

impl Default for FieldMap {
    fn default() -> Self {
        Self::new(
            &[
                "level",
                "lvl",
                "severity",
                "levelname",
                "loglevel",
                "log_level",
                "log.level",
            ],
            &[
                "time",
                "ts",
                "timestamp",
                "@timestamp",
                "datetime",
                "asctime",
            ],
            &["msg", "message", "text", "event"],
        )
    }
}

impl FieldMap {
    /// A map from the three lists of key names (most specific first).
    pub fn new(level: &[&str], time: &[&str], message: &[&str]) -> Self {
        let own = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect();
        Self {
            level: own(level),
            time: own(time),
            message: own(message),
        }
    }

    /// The default map with `level`, `time` and `message` names tried before the defaults.
    pub fn with_first(mut self, level: &[&str], time: &[&str], message: &[&str]) -> Self {
        let put_first = |names: &mut Vec<String>, first: &[&str]| {
            let mut all: Vec<String> = first.iter().map(|name| (*name).to_owned()).collect();
            all.append(names);
            *names = all;
        };
        put_first(&mut self.level, level);
        put_first(&mut self.time, time);
        put_first(&mut self.message, message);
        self
    }
}
