use crate::domain::GeoLocation;
use chrono_tz::Tz;
use config::{Config, FileFormat};
use serde::Deserialize;
use std::time::Duration;
use tracing::warn;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    core: Core,
    db: Db,
    mqtt: Mqtt,
    hue: Hue,
    shelly: Shelly,
    timezone: Option<Tz>,
    location: GeoLocation,
}

impl AppConfig {
    pub fn load() -> Self {
        const DEFAULT_CONFIG: &str = include_str!("../config.json5");

        Config::builder()
            .add_source(config::File::from_str(DEFAULT_CONFIG, FileFormat::Json5))
            .add_source(config::File::with_name("config.local").required(false))
            .add_source(config::Environment::default())
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub fn db(&self) -> &Db {
        &self.db
    }

    pub fn mqtt(&self) -> &Mqtt {
        &self.mqtt
    }

    pub fn hue(&self) -> &Hue {
        &self.hue
    }

    pub fn shelly(&self) -> &Shelly {
        &self.shelly
    }

    // Defaults to the system timezone. Note that chrono::Local isn't used for this as it wrongly treats 02:00 on
    // fall-back day as occurring twice and returns the two candidates in the wrong order
    // (https://github.com/chronotope/chrono/issues/1625), which makes cron yield the same time forever.
    pub fn timezone(&self) -> Tz {
        self.timezone.unwrap_or_else(|| {
            iana_time_zone::get_timezone().ok().and_then(|name| name.parse().ok()).unwrap_or_else(|| {
                warn!("🕗 Unable to determine the system timezone, falling back to UTC");
                Tz::UTC
            })
        })
    }

    pub fn geo_location(&self) -> &GeoLocation {
        &self.location
    }
}

#[derive(Debug, Deserialize)]
pub struct Core {
    port: usize,
    store_buffer_size: usize,
    client_connection_timeout_ms: u64,
    client_request_timeout_ms: u64,
}

impl Core {
    pub fn port(&self) -> usize {
        self.port
    }

    pub fn store_buffer_size(&self) -> usize {
        self.store_buffer_size
    }

    pub fn client_connection_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.client_connection_timeout_ms)
    }

    pub fn client_request_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.client_request_timeout_ms)
    }
}

#[derive(Debug, Deserialize)]
pub struct Db {
    file_name: String,
}

impl Db {
    pub fn file_name(&self) -> &str {
        &self.file_name
    }
}

#[derive(Debug, Deserialize)]
pub struct Mqtt {
    enabled: bool,
    host: String,
    port: usize,
    advertised_host: String,
    advertised_port: usize,
    username: String,
    password: String,
}

impl Mqtt {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> usize {
        self.port
    }

    pub fn advertised_host(&self) -> &str {
        &self.advertised_host
    }

    pub fn advertised_port(&self) -> usize {
        self.advertised_port
    }

    pub fn username(&self) -> &str {
        &self.username
    }

    pub fn password(&self) -> &str {
        &self.password
    }
}

#[derive(Debug, Deserialize)]
pub struct Hue {
    enabled: bool,
    url: String,
    retry_ms: u64,
    retry_max_delay_ms: u64,
    stale_connection_timeout_ms: u64,
    send_timeout_ms: u64,
    application_key: String,
}

impl Hue {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn retry_ms(&self) -> u64 {
        self.retry_ms
    }

    pub fn retry_max_delay_ms(&self) -> Duration {
        Duration::from_millis(self.retry_max_delay_ms)
    }

    pub fn stale_connection_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.stale_connection_timeout_ms)
    }

    pub fn send_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.send_timeout_ms)
    }

    pub fn application_key(&self) -> &str {
        &self.application_key
    }
}

#[derive(Debug, Deserialize)]
pub struct Shelly {
    enabled: bool,
    client_connection_timeout_ms: u64,
    client_request_timeout_ms: u64,
}

impl Shelly {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn client_connection_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.client_connection_timeout_ms)
    }

    pub fn client_request_timeout_ms(&self) -> Duration {
        Duration::from_millis(self.client_request_timeout_ms)
    }
}

#[cfg(test)]
pub struct AppConfigBuilder {
    config: AppConfig,
}

#[cfg(test)]
impl AppConfigBuilder {
    pub fn new() -> Self {
        AppConfigBuilder {
            config: AppConfig {
                core: Core {
                    port: 8080,
                    store_buffer_size: 1,
                    client_connection_timeout_ms: 100,
                    client_request_timeout_ms: 100,
                },
                db: Db { file_name: "hearth.db".to_string() },
                mqtt: Mqtt {
                    enabled: true,
                    host: "127.0.0.1".to_string(),
                    port: 1883,
                    advertised_host: "192.168.1.100".to_string(),
                    advertised_port: 1883,
                    username: "hearth".to_string(),
                    password: "p4ssY".to_string(),
                },
                hue: Hue {
                    enabled: true,
                    url: "https://hue.url/".to_string(),
                    retry_ms: 100,
                    retry_max_delay_ms: 200,
                    stale_connection_timeout_ms: 30_000,
                    send_timeout_ms: 100,
                    application_key: "key".to_string(),
                },
                shelly: Shelly { enabled: true, client_connection_timeout_ms: 100, client_request_timeout_ms: 100 },
                timezone: Some(Tz::Europe__Amsterdam),
                location: GeoLocation { latitude: 51.8615899, longitude: 4.3580323, altitude: 0.0 },
            },
        }
    }

    pub fn hue_enabled(mut self, enabled: bool) -> Self {
        self.config.hue.enabled = enabled;
        self
    }

    pub fn hue_url(mut self, url: String) -> Self {
        self.config.hue.url = url;
        self
    }

    pub fn build(self) -> AppConfig {
        self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    fn load_with_overrides(overrides: &str) -> Result<AppConfig, config::ConfigError> {
        Config::builder()
            .add_source(config::File::from_str(include_str!("../config.json5"), FileFormat::Json5))
            .add_source(config::File::from_str(overrides, FileFormat::Json5))
            .build()?
            .try_deserialize()
    }

    #[rstest]
    #[case::europe_london("Europe/London", Tz::Europe__London)]
    #[case::utc("UTC", Tz::UTC)]
    fn timezone_uses_the_configured_value(#[case] name: &str, #[case] expected: Tz) {
        let config = load_with_overrides(&format!(r#"{{ "timezone": "{name}" }}"#)).unwrap();

        assert_eq!(config.timezone(), expected);
    }

    #[test]
    fn loading_fails_for_an_unknown_timezone() {
        let result = load_with_overrides(r#"{ "timezone": "Europe/Atlantis" }"#);

        assert!(result.is_err());
    }

    #[test]
    fn loading_without_a_timezone_uses_the_system_timezone() {
        let config = load_with_overrides("{}").unwrap();

        assert_eq!(config.timezone, None);
    }
}
