//! In-memory Prometheus text-format metrics (no external crates).

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
struct Data {
    /// (client, kind, status) -> count
    requests: HashMap<(String, String, u16), u64>,
    /// kind -> (count, total_ms)
    duration: HashMap<String, (u64, u64)>,
    upstream_errors: HashMap<String, u64>,
}

#[derive(Default)]
pub struct Metrics {
    data: Mutex<Data>,
}

impl Metrics {
    pub fn record_request(&self, client: &str, kind: &str, status: u16, ms: u64) {
        let mut d = self.data.lock().expect("poisoned");
        *d.requests
            .entry((client.to_owned(), kind.to_owned(), status))
            .or_insert(0) += 1;
        let e = d.duration.entry(kind.to_owned()).or_insert((0, 0));
        e.0 += 1;
        e.1 += ms;
    }

    pub fn record_upstream_error(&self, kind: &str) {
        let mut d = self.data.lock().expect("poisoned");
        *d.upstream_errors.entry(kind.to_owned()).or_insert(0) += 1;
    }

    pub fn render(&self) -> String {
        let d = self.data.lock().expect("poisoned");
        let mut out = String::new();
        out.push_str("# TYPE tgb_requests_total counter\n");
        for ((client, kind, status), count) in &d.requests {
            out.push_str(&format!(
                "tgb_requests_total{{client=\"{client}\",kind=\"{kind}\",status=\"{status}\"}} {count}\n"
            ));
        }
        out.push_str("# TYPE tgb_request_duration_milliseconds summary\n");
        for (kind, (count, sum)) in &d.duration {
            out.push_str(&format!(
                "tgb_request_duration_milliseconds_count{{kind=\"{kind}\"}} {count}\n"
            ));
            out.push_str(&format!(
                "tgb_request_duration_milliseconds_sum{{kind=\"{kind}\"}} {sum}\n"
            ));
        }
        if !d.upstream_errors.is_empty() {
            out.push_str("# TYPE tgb_upstream_errors_total counter\n");
            for (kind, count) in &d.upstream_errors {
                out.push_str(&format!(
                    "tgb_upstream_errors_total{{kind=\"{kind}\"}} {count}\n"
                ));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_prometheus_text() {
        let m = Metrics::default();
        m.record_request("c1", "passthrough", 200, 42);
        m.record_request("c1", "passthrough", 200, 58);
        m.record_upstream_error("action");
        let s = m.render();
        assert!(s.contains(
            "tgb_requests_total{client=\"c1\",kind=\"passthrough\",status=\"200\"} 2"
        ));
        assert!(s.contains("tgb_request_duration_milliseconds_sum{kind=\"passthrough\"} 100"));
        assert!(s.contains("tgb_upstream_errors_total{kind=\"action\"} 1"));
    }
}
