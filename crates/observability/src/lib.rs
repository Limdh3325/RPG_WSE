use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

/// Standard Prometheus latency buckets for Tick duration (in seconds).
pub const TICK_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.010, 0.020, 0.030, 0.050, 0.075, 0.100, 0.250, 0.500, 1.0, 2.5,
];

/// Standard Prometheus latency buckets for Persistence latency (in seconds).
pub const PERSISTENCE_LATENCY_BUCKETS: &[f64] = &[
    0.005, 0.015, 0.030, 0.050, 0.100, 0.150, 0.300, 0.500, 1.0, 2.0,
];

/// Standard Prometheus latency buckets for Resync latency (in seconds).
pub const RESYNC_LATENCY_BUCKETS: &[f64] = &[0.050, 0.100, 0.250, 0.500, 1.0, 1.5, 2.0, 3.0, 5.0];

#[derive(Debug, Clone)]
pub struct HistogramData {
    pub buckets: Vec<f64>,
    pub counts: Vec<u64>,
    pub count: u64,
    pub sum: f64,
    pub raw_values: Vec<f64>,
}

impl HistogramData {
    pub fn new(buckets: &[f64]) -> Self {
        Self {
            buckets: buckets.to_vec(),
            counts: vec![0; buckets.len()],
            count: 0,
            sum: 0.0,
            raw_values: Vec::new(),
        }
    }

    pub fn observe(&mut self, val: f64) {
        self.count += 1;
        self.sum += val;
        self.raw_values.push(val);
        for (i, &upper) in self.buckets.iter().enumerate() {
            if val <= upper {
                self.counts[i] += 1;
            }
        }
    }

    pub fn percentile(&self, p: f64) -> f64 {
        if self.raw_values.is_empty() {
            return 0.0;
        }
        let mut sorted = self.raw_values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rank = ((sorted.len() as f64) * p).ceil() as usize;
        let idx = if rank == 0 {
            0
        } else if rank > sorted.len() {
            sorted.len() - 1
        } else {
            rank - 1
        };
        sorted[idx]
    }
}

#[derive(Default)]
struct MetricsInner {
    // Histograms
    tick_durations: BTreeMap<(String, String), HistogramData>, // (region, mode)
    persistence_latencies: BTreeMap<String, HistogramData>,    // operation
    resync_latencies: BTreeMap<String, HistogramData>,         // reason

    // Counters
    tick_overruns: BTreeMap<String, u64>,           // region
    npc_decisions: BTreeMap<(String, String), u64>, // (region, goal_kind)
    goap_expansions: BTreeMap<String, u64>,         // region
    command_rejections: BTreeMap<(String, String), u64>, // (region, reason)
    region_epoch_changes: BTreeMap<String, u64>,    // region
    replication_bytes: BTreeMap<(String, String), u64>, // (region, message_type)
    client_resyncs: BTreeMap<String, u64>,          // reason

    // Gauges
    region_entities: BTreeMap<(String, String), i64>, // (region, entity_kind)
    command_queue_depths: BTreeMap<(String, String), i64>, // (region, queue_type)
    snapshot_ages: BTreeMap<String, f64>,             // region
}

#[derive(Clone, Default)]
pub struct WseMetricsRegistry {
    inner: Arc<RwLock<MetricsInner>>,
}

static GLOBAL_REGISTRY: OnceLock<WseMetricsRegistry> = OnceLock::new();

impl WseMetricsRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(MetricsInner::default())),
        }
    }

    pub fn global() -> &'static Self {
        GLOBAL_REGISTRY.get_or_init(Self::new)
    }

    pub fn record_tick_duration(&self, region: &str, mode: &str, duration_secs: f64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        let key = (region.to_string(), mode.to_string());
        guard
            .tick_durations
            .entry(key)
            .or_insert_with(|| HistogramData::new(TICK_DURATION_BUCKETS))
            .observe(duration_secs);
    }

    pub fn record_tick_overrun(&self, region: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard.tick_overruns.entry(region.to_string()).or_insert(0) += 1;
    }

    pub fn set_region_entities(&self, region: &str, entity_kind: &str, count: i64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard
            .region_entities
            .insert((region.to_string(), entity_kind.to_string()), count);
    }

    pub fn record_npc_decision(&self, region: &str, goal_kind: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard
            .npc_decisions
            .entry((region.to_string(), goal_kind.to_string()))
            .or_insert(0) += 1;
    }

    pub fn record_goap_expansions(&self, region: &str, count: u64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard.goap_expansions.entry(region.to_string()).or_insert(0) += count;
    }

    pub fn set_command_queue_depth(&self, region: &str, queue_type: &str, depth: i64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard
            .command_queue_depths
            .insert((region.to_string(), queue_type.to_string()), depth);
    }

    pub fn record_command_rejection(&self, region: &str, reason: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard
            .command_rejections
            .entry((region.to_string(), reason.to_string()))
            .or_insert(0) += 1;
    }

    pub fn record_persistence_latency(&self, operation: &str, duration_secs: f64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard
            .persistence_latencies
            .entry(operation.to_string())
            .or_insert_with(|| HistogramData::new(PERSISTENCE_LATENCY_BUCKETS))
            .observe(duration_secs);
    }

    pub fn record_resync_latency(&self, reason: &str, duration_secs: f64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard
            .resync_latencies
            .entry(reason.to_string())
            .or_insert_with(|| HistogramData::new(RESYNC_LATENCY_BUCKETS))
            .observe(duration_secs);
    }

    pub fn set_snapshot_age(&self, region: &str, age_secs: f64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        guard.snapshot_ages.insert(region.to_string(), age_secs);
    }

    pub fn record_region_epoch_change(&self, region: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard
            .region_epoch_changes
            .entry(region.to_string())
            .or_insert(0) += 1;
    }

    pub fn record_replication_bytes(&self, region: &str, message_type: &str, bytes: u64) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard
            .replication_bytes
            .entry((region.to_string(), message_type.to_string()))
            .or_insert(0) += bytes;
    }

    pub fn record_client_resync(&self, reason: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|p| p.into_inner());
        *guard.client_resyncs.entry(reason.to_string()).or_insert(0) += 1;
    }

    pub fn get_tick_percentiles(&self, region: &str, mode: &str) -> (f64, f64, f64) {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        let key = (region.to_string(), mode.to_string());
        if let Some(hist) = guard.tick_durations.get(&key) {
            (
                hist.percentile(0.50),
                hist.percentile(0.95),
                hist.percentile(0.99),
            )
        } else {
            (0.0, 0.0, 0.0)
        }
    }

    pub fn get_persistence_percentiles(&self, operation: &str) -> (f64, f64, f64) {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        if let Some(hist) = guard.persistence_latencies.get(operation) {
            (
                hist.percentile(0.50),
                hist.percentile(0.95),
                hist.percentile(0.99),
            )
        } else {
            (0.0, 0.0, 0.0)
        }
    }

    pub fn get_resync_percentiles(&self, reason: &str) -> (f64, f64, f64) {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        if let Some(hist) = guard.resync_latencies.get(reason) {
            (
                hist.percentile(0.50),
                hist.percentile(0.95),
                hist.percentile(0.99),
            )
        } else {
            (0.0, 0.0, 0.0)
        }
    }

    /// Renders standard Prometheus text scrape output.
    pub fn render_prometheus_text(&self) -> String {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        let mut out = String::with_capacity(4096);

        // 1. wse_tick_duration_seconds (Histogram)
        out.push_str("# HELP wse_tick_duration_seconds Duration of simulation ticks in seconds.\n");
        out.push_str("# TYPE wse_tick_duration_seconds histogram\n");
        for ((region, mode), hist) in &guard.tick_durations {
            for (i, &upper) in hist.buckets.iter().enumerate() {
                out.push_str(&format!(
                    "wse_tick_duration_seconds_bucket{{region=\"{}\",mode=\"{}\",le=\"{}\"}} {}\n",
                    escape_label(region),
                    escape_label(mode),
                    upper,
                    hist.counts[i]
                ));
            }
            out.push_str(&format!(
                "wse_tick_duration_seconds_bucket{{region=\"{}\",mode=\"{}\",le=\"+Inf\"}} {}\n",
                escape_label(region),
                escape_label(mode),
                hist.count
            ));
            out.push_str(&format!(
                "wse_tick_duration_seconds_sum{{region=\"{}\",mode=\"{}\"}} {:.6}\n",
                escape_label(region),
                escape_label(mode),
                hist.sum
            ));
            out.push_str(&format!(
                "wse_tick_duration_seconds_count{{region=\"{}\",mode=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(mode),
                hist.count
            ));
        }

        // 2. wse_tick_overruns_total (Counter)
        out.push_str("# HELP wse_tick_overruns_total Total number of tick deadline overruns.\n");
        out.push_str("# TYPE wse_tick_overruns_total counter\n");
        for (region, count) in &guard.tick_overruns {
            out.push_str(&format!(
                "wse_tick_overruns_total{{region=\"{}\"}} {}\n",
                escape_label(region),
                count
            ));
        }

        // 3. wse_region_entities (Gauge)
        out.push_str("# HELP wse_region_entities Current number of entities in region by kind.\n");
        out.push_str("# TYPE wse_region_entities gauge\n");
        for ((region, kind), count) in &guard.region_entities {
            out.push_str(&format!(
                "wse_region_entities{{region=\"{}\",entity_kind=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(kind),
                count
            ));
        }

        // 4. wse_npc_decisions_total (Counter)
        out.push_str("# HELP wse_npc_decisions_total Total NPC AI decision cycles evaluated.\n");
        out.push_str("# TYPE wse_npc_decisions_total counter\n");
        for ((region, goal_kind), count) in &guard.npc_decisions {
            out.push_str(&format!(
                "wse_npc_decisions_total{{region=\"{}\",goal_kind=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(goal_kind),
                count
            ));
        }

        // 5. wse_goap_expansions_total (Counter)
        out.push_str(
            "# HELP wse_goap_expansions_total Total GOAP planner search node expansions.\n",
        );
        out.push_str("# TYPE wse_goap_expansions_total counter\n");
        for (region, count) in &guard.goap_expansions {
            out.push_str(&format!(
                "wse_goap_expansions_total{{region=\"{}\"}} {}\n",
                escape_label(region),
                count
            ));
        }

        // 6. wse_command_queue_depth (Gauge)
        out.push_str("# HELP wse_command_queue_depth Current command queue depths.\n");
        out.push_str("# TYPE wse_command_queue_depth gauge\n");
        for ((region, q_type), depth) in &guard.command_queue_depths {
            out.push_str(&format!(
                "wse_command_queue_depth{{region=\"{}\",queue_type=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(q_type),
                depth
            ));
        }

        // 7. wse_command_rejections_total (Counter)
        out.push_str("# HELP wse_command_rejections_total Total rejected client commands.\n");
        out.push_str("# TYPE wse_command_rejections_total counter\n");
        for ((region, reason), count) in &guard.command_rejections {
            out.push_str(&format!(
                "wse_command_rejections_total{{region=\"{}\",reason=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(reason),
                count
            ));
        }

        // 8. wse_persistence_latency_seconds (Histogram)
        out.push_str(
            "# HELP wse_persistence_latency_seconds Latency of storage operations in seconds.\n",
        );
        out.push_str("# TYPE wse_persistence_latency_seconds histogram\n");
        for (op, hist) in &guard.persistence_latencies {
            for (i, &upper) in hist.buckets.iter().enumerate() {
                out.push_str(&format!(
                    "wse_persistence_latency_seconds_bucket{{operation=\"{}\",le=\"{}\"}} {}\n",
                    escape_label(op),
                    upper,
                    hist.counts[i]
                ));
            }
            out.push_str(&format!(
                "wse_persistence_latency_seconds_bucket{{operation=\"{}\",le=\"+Inf\"}} {}\n",
                escape_label(op),
                hist.count
            ));
            out.push_str(&format!(
                "wse_persistence_latency_seconds_sum{{operation=\"{}\"}} {:.6}\n",
                escape_label(op),
                hist.sum
            ));
            out.push_str(&format!(
                "wse_persistence_latency_seconds_count{{operation=\"{}\"}} {}\n",
                escape_label(op),
                hist.count
            ));
        }

        // 9. wse_snapshot_age_seconds (Gauge)
        out.push_str(
            "# HELP wse_snapshot_age_seconds Age of latest durable snapshot in seconds.\n",
        );
        out.push_str("# TYPE wse_snapshot_age_seconds gauge\n");
        for (region, age) in &guard.snapshot_ages {
            out.push_str(&format!(
                "wse_snapshot_age_seconds{{region=\"{}\"}} {:.3}\n",
                escape_label(region),
                age
            ));
        }

        // 10. wse_region_epoch_changes_total (Counter)
        out.push_str("# HELP wse_region_epoch_changes_total Total epoch ownership transitions.\n");
        out.push_str("# TYPE wse_region_epoch_changes_total counter\n");
        for (region, count) in &guard.region_epoch_changes {
            out.push_str(&format!(
                "wse_region_epoch_changes_total{{region=\"{}\"}} {}\n",
                escape_label(region),
                count
            ));
        }

        // 11. wse_replication_bytes_total (Counter)
        out.push_str("# HELP wse_replication_bytes_total Total replicated network bytes.\n");
        out.push_str("# TYPE wse_replication_bytes_total counter\n");
        for ((region, msg_type), bytes) in &guard.replication_bytes {
            out.push_str(&format!(
                "wse_replication_bytes_total{{region=\"{}\",message_type=\"{}\"}} {}\n",
                escape_label(region),
                escape_label(msg_type),
                bytes
            ));
        }

        // 12. wse_client_resync_total (Counter)
        out.push_str("# HELP wse_client_resync_total Total client resync requests handled.\n");
        out.push_str("# TYPE wse_client_resync_total counter\n");
        for (reason, count) in &guard.client_resyncs {
            out.push_str(&format!(
                "wse_client_resync_total{{reason=\"{}\"}} {}\n",
                escape_label(reason),
                count
            ));
        }

        out
    }

    /// Helper to compute p95 persistence latency for an operation.
    pub fn persistence_latency_p95(&self, operation: &str) -> f64 {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        guard
            .persistence_latencies
            .get(operation)
            .map(|h| h.percentile(0.95))
            .unwrap_or(0.0)
    }

    /// Helper to compute p95 tick duration for a region and mode.
    pub fn tick_duration_p95(&self, region: &str, mode: &str) -> f64 {
        let guard = self.inner.read().unwrap_or_else(|p| p.into_inner());
        guard
            .tick_durations
            .get(&(region.to_string(), mode.to_string()))
            .map(|h| h.percentile(0.95))
            .unwrap_or(0.0)
    }
}

fn escape_label(val: &str) -> String {
    val.replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_registry_recording_and_prometheus_export() {
        let registry = WseMetricsRegistry::new();

        registry.record_tick_duration("settlement_main", "active", 0.012);
        registry.record_tick_duration("settlement_main", "active", 0.025);
        registry.record_tick_overrun("settlement_main");
        registry.set_region_entities("settlement_main", "npc", 1000);
        registry.set_region_entities("settlement_main", "player", 10);
        registry.record_npc_decision("settlement_main", "patrol");
        registry.record_goap_expansions("settlement_main", 42);
        registry.set_command_queue_depth("settlement_main", "inbound", 3);
        registry.record_command_rejection("settlement_main", "invalid_epoch");
        registry.record_persistence_latency("commit_transaction", 0.045);
        registry.record_resync_latency("sequence_gap", 0.250);
        registry.set_snapshot_age("settlement_main", 15.2);
        registry.record_region_epoch_change("settlement_main");
        registry.record_replication_bytes("settlement_main", "WorldSnapshot", 2048);
        registry.record_client_resync("sequence_gap");

        let text = registry.render_prometheus_text();

        assert!(text.contains("wse_tick_duration_seconds_bucket{region=\"settlement_main\",mode=\"active\",le=\"0.02\"} 1"));
        assert!(text.contains("wse_tick_overruns_total{region=\"settlement_main\"} 1"));
        assert!(
            text.contains(
                "wse_region_entities{region=\"settlement_main\",entity_kind=\"npc\"} 1000"
            )
        );
        assert!(
            text.contains(
                "wse_region_entities{region=\"settlement_main\",entity_kind=\"player\"} 10"
            )
        );
        assert!(text.contains("wse_goap_expansions_total{region=\"settlement_main\"} 42"));
        assert!(text.contains(
            "wse_command_queue_depth{region=\"settlement_main\",queue_type=\"inbound\"} 3"
        ));
        assert!(text.contains(
            "wse_command_rejections_total{region=\"settlement_main\",reason=\"invalid_epoch\"} 1"
        ));
        assert!(
            text.contains(
                "wse_persistence_latency_seconds_count{operation=\"commit_transaction\"} 1"
            )
        );
        assert!(text.contains("wse_snapshot_age_seconds{region=\"settlement_main\"} 15.200"));
        assert!(text.contains("wse_region_epoch_changes_total{region=\"settlement_main\"} 1"));
        assert!(text.contains("wse_replication_bytes_total{region=\"settlement_main\",message_type=\"WorldSnapshot\"} 2048"));
        assert!(text.contains("wse_client_resync_total{reason=\"sequence_gap\"} 1"));

        let (p50, p95, p99) = registry.get_tick_percentiles("settlement_main", "active");
        assert!(p50 > 0.010 && p50 <= 0.025);
        assert!(p95 > 0.020 && p95 <= 0.025);
        assert!(p99 > 0.020 && p99 <= 0.025);
    }
}
