use config::Config;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub hue: Hue,
}

impl AppConfig {
    pub fn load() -> Self {
        Config::builder()
            .add_source(config::File::with_name("config").required(true))
            .add_source(config::Environment::default())
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    pub fn hue(&self) -> &Hue {
        &self.hue
    }
}

#[derive(Debug, Deserialize)]
pub struct Hue {
    pub url: String,
    pub retry_ms: u64,
    pub max_delay_ms: u64,
    pub application_key: String,
}

impl Hue {
    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn retry_ms(&self) -> u64 {
        self.retry_ms
    }

    pub fn max_delay_ms(&self) -> Duration {
        Duration::from_millis(self.max_delay_ms)
    }

    pub fn application_key(&self) -> &str {
        &self.application_key
    }
}
