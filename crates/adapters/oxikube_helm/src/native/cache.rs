//! Decoded revisions cached by `(uid, resourceVersion)`: a stored revision whose
//! `resourceVersion` is unchanged has the same bytes, so it is never decoded twice.
//!
//! Headers are small and every list needs them, so many are kept; full records (values and
//! manifests, possibly MBs) are few and least-recently-used. Memory only (non-negotiable 5).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::record::{Header, Record};
use super::storage::CacheKey;

/// Headers kept before the map is dropped and refilled (a 500-release cluster with ten
/// revisions each fits).
const MAX_HEADERS: usize = 8192;

/// Full records kept, most recently used last.
const MAX_RECORDS: usize = 8;

/// The adapter's decode cache.
#[derive(Default)]
pub(crate) struct DecodeCache {
    headers: Mutex<HashMap<CacheKey, Arc<Header>>>,
    records: Mutex<VecDeque<(CacheKey, Arc<Record>)>>,
}

impl DecodeCache {
    pub fn header(&self, key: &CacheKey) -> Option<Arc<Header>> {
        if let Some(header) = lock(&self.headers).get(key) {
            return Some(header.clone());
        }
        // A decoded full record has the header too.
        lock(&self.records)
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, record)| Arc::new(record.header.clone()))
    }

    pub fn put_header(&self, key: CacheKey, header: Arc<Header>) {
        let mut headers = lock(&self.headers);
        if headers.len() >= MAX_HEADERS {
            headers.clear();
        }
        headers.insert(key, header);
    }

    pub fn record(&self, key: &CacheKey) -> Option<Arc<Record>> {
        let mut records = lock(&self.records);
        let at = records.iter().position(|(k, _)| k == key)?;
        let entry = records.remove(at)?;
        let record = entry.1.clone();
        records.push_back(entry);
        Some(record)
    }

    pub fn put_record(&self, key: CacheKey, record: Arc<Record>) {
        let mut records = lock(&self.records);
        records.retain(|(k, _)| *k != key);
        if records.len() >= MAX_RECORDS {
            records.pop_front();
        }
        records.push_back((key, record));
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use oxikube_domain::helm::{ChartRef, Values};

    use super::*;

    fn key(n: usize) -> CacheKey {
        (Arc::from(format!("uid-{n}")), Arc::from("1"))
    }

    fn record(description: &str) -> Arc<Record> {
        Arc::new(Record {
            header: Header {
                chart: ChartRef::new("demo", "1.0.0"),
                app_version: None,
                status: None,
                updated: None,
                description: description.into(),
            },
            notes: None,
            user_values: Values::default(),
            chart_values: None,
            manifest: Arc::from(""),
        })
    }

    #[test]
    fn records_are_least_recently_used() {
        let cache = DecodeCache::default();
        for n in 0..MAX_RECORDS {
            cache.put_record(key(n), record(&n.to_string()));
        }
        assert!(cache.record(&key(0)).is_some(), "touch 0");
        cache.put_record(key(MAX_RECORDS), record("new"));
        assert!(cache.record(&key(0)).is_some(), "recently used survives");
        assert!(cache.record(&key(1)).is_none(), "least recently used goes");
    }

    #[test]
    fn a_record_serves_its_header() {
        let cache = DecodeCache::default();
        cache.put_record(key(1), record("Install complete"));
        assert_eq!(
            cache.header(&key(1)).unwrap().description,
            "Install complete"
        );
        assert!(cache.header(&key(2)).is_none());
    }
}
