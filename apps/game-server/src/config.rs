use serde::Deserialize;
use std::{collections::BTreeMap, net::SocketAddr, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read configuration: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid configuration TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("invalid configuration field: {0}")]
    Invalid(String),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub server: Server,
    pub simulation: Simulation,
    pub region: Region,
    pub network: Network,
    pub persistence: Persistence,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub http_listen: SocketAddr,
    pub ws_auth_timeout_seconds: u64,
    pub protocol_version: u32,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Simulation {
    pub tick_rate_hz: u32,
    pub game_time_scale: u32,
    pub npc_decision_interval_ticks: u32,
    pub npc_long_planning_interval_ticks: u32,
    pub max_npc_evaluations_per_tick: usize,
    pub max_goap_expansions_per_plan: usize,
    pub max_events_per_tick: usize,
    pub max_causation_depth: usize,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    pub command_queue_capacity: usize,
    pub persistence_ack_queue_capacity: usize,
    pub durable_queue_capacity: usize,
    pub transient_delta_queue_capacity: usize,
    pub lease_seconds: u64,
    pub lease_renew_seconds: u64,
    pub snapshot_interval_seconds: u64,
    pub active_to_background_grace_seconds: u64,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Network {
    pub aoi_cell_tile_size: u32,
    pub aoi_radius_cells: u32,
    pub state_delta_hz: u32,
    pub max_message_bytes: usize,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Persistence {
    pub pool_max_connections: u32,
    pub transaction_timeout_ms: u64,
    pub outbox_batch_size: u32,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::parse(&std::fs::read_to_string(path)?, &std::env::vars().collect())
    }

    pub fn parse(text: &str, env: &BTreeMap<String, String>) -> Result<Self, ConfigError> {
        let mut c: Self = toml::from_str(text)?;
        macro_rules! override_field {
            ($name:literal, $field:expr) => {
                if let Some(value) = env.get($name) {
                    $field = value
                        .parse()
                        .map_err(|_| ConfigError::Invalid($name.into()))?;
                }
            };
        }
        override_field!("WSE_HTTP_LISTEN", c.server.http_listen);
        override_field!("WSE_WS_AUTH_TIMEOUT_SEC", c.server.ws_auth_timeout_seconds);
        override_field!("WSE_TICK_RATE_HZ", c.simulation.tick_rate_hz);
        override_field!("WSE_GAME_TIME_SCALE", c.simulation.game_time_scale);
        override_field!("WSE_REGION_CMD_CAP", c.region.command_queue_capacity);
        override_field!("WSE_DURABLE_CAP", c.region.durable_queue_capacity);
        override_field!(
            "WSE_SNAPSHOT_INTERVAL_SEC",
            c.region.snapshot_interval_seconds
        );
        override_field!("WSE_REGION_LEASE_SEC", c.region.lease_seconds);
        override_field!("WSE_LEASE_RENEW_SEC", c.region.lease_renew_seconds);
        c.validate()?;
        Ok(c)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        macro_rules! positive {
            ($($field:expr),+ $(,)?) => {
                $(if $field == 0 { return Err(ConfigError::Invalid(stringify!($field).into())); })+
            };
        }
        positive!(
            self.server.ws_auth_timeout_seconds,
            self.simulation.tick_rate_hz,
            self.simulation.game_time_scale,
            self.simulation.npc_decision_interval_ticks,
            self.simulation.npc_long_planning_interval_ticks,
            self.simulation.max_npc_evaluations_per_tick,
            self.simulation.max_goap_expansions_per_plan,
            self.simulation.max_events_per_tick,
            self.simulation.max_causation_depth,
            self.region.command_queue_capacity,
            self.region.persistence_ack_queue_capacity,
            self.region.durable_queue_capacity,
            self.region.transient_delta_queue_capacity,
            self.region.lease_seconds,
            self.region.lease_renew_seconds,
            self.region.snapshot_interval_seconds,
            self.region.active_to_background_grace_seconds,
            self.network.aoi_cell_tile_size,
            self.network.state_delta_hz,
            self.network.max_message_bytes,
            self.persistence.pool_max_connections,
            self.persistence.transaction_timeout_ms,
            self.persistence.outbox_batch_size,
        );
        if self.server.protocol_version != 1 {
            return Err(ConfigError::Invalid(
                "server.protocol_version: only 1 is supported".into(),
            ));
        }
        if self.simulation.tick_rate_hz > 1000 || 1000 % self.simulation.tick_rate_hz != 0 {
            return Err(ConfigError::Invalid(
                "tick_rate_hz must divide 1000ms".into(),
            ));
        }
        if self.region.lease_renew_seconds >= self.region.lease_seconds {
            return Err(ConfigError::Invalid(
                "lease renewal must precede expiry".into(),
            ));
        }
        if self.network.state_delta_hz > self.simulation.tick_rate_hz {
            return Err(ConfigError::Invalid(
                "state_delta_hz exceeds tick rate".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const SAMPLE: &str = include_str!("../../../schemas/config.example.toml");

    #[test]
    fn sample_and_environment_precedence() {
        let env = BTreeMap::from([
            ("WSE_HTTP_LISTEN".into(), "127.0.0.1:19080".into()),
            ("WSE_REGION_CMD_CAP".into(), "128".into()),
        ]);
        let c = Config::parse(SAMPLE, &env).unwrap();
        assert_eq!(c.server.http_listen.port(), 19080);
        assert_eq!(c.region.command_queue_capacity, 128);
        assert_eq!(c.simulation.tick_rate_hz, 10);
    }
    #[test]
    fn rejects_invalid_override_and_timing() {
        for (key, value) in [
            ("WSE_TICK_RATE_HZ", "0"),
            ("WSE_TICK_RATE_HZ", "3"),
            ("WSE_REGION_CMD_CAP", "0"),
            ("WSE_REGION_LEASE_SEC", "5"),
            ("WSE_HTTP_LISTEN", "bad"),
            ("WSE_GAME_TIME_SCALE", "bad"),
        ] {
            let env = BTreeMap::from([(key.into(), value.into())]);
            assert!(Config::parse(SAMPLE, &env).is_err(), "{key}");
        }
    }
    #[test]
    fn rejects_typo_and_unsupported_protocol() {
        assert!(
            Config::parse(
                &SAMPLE.replace("protocol_version = 1", "protocol_version = 2"),
                &BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            Config::parse(
                &SAMPLE.replace("tick_rate_hz", "tick_rate_typo"),
                &BTreeMap::new()
            )
            .is_err()
        );
    }
}
