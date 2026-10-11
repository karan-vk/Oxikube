# Recorded responses

Bodies recorded with `curl` from real servers started in Docker, one file per case, named
`<product>-<version>-<case>.json`:

| Product | Version | Notes |
|---|---|---|
| Prometheus | 3.5.0 | scraping itself every second |
| VictoriaMetrics | 1.122.0 | `-selfScrapeInterval=1s`; it drops `NaN`, answers `422` with `"errorType":"422"` for a parse error, and does not reject an inverted range |
| Mimir | 2.17.0 | monolithic, single tenant, fed by Prometheus remote write; served under `/prometheus` |

Cases: `instant-vector`, `range-matrix`, `instant-scalar`, `instant-empty`, `instant-nan`,
`instant-inf`, and `error-bad-data` / `error-execution` / `error-bad-range` (the HTTP status each
server used is in `tests/recorded.rs`). Timestamps are from the day of recording; the tests do not
depend on them being recent. Re-record by running the same containers and `curl`ing
`/api/v1/query` and `/api/v1/query_range`.
