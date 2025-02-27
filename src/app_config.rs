use config::Config;
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    core: Core,
    flows: Flows,
    hue: Hue,
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

    pub fn core(&self) -> &Core {
        &self.core
    }

    pub fn flows(&self) -> &Flows {
        &self.flows
    }

    pub fn hue(&self) -> &Hue {
        &self.hue
    }
}

#[derive(Debug, Deserialize)]
pub struct Core {
    store_buffer_size: usize,
}

impl Core {
    pub fn store_buffer_size(&self) -> usize {
        self.store_buffer_size
    }
}

#[derive(Debug, Deserialize)]
pub struct Flows {
    directory: String,
}

impl Flows {
    pub fn directory(&self) -> &str {
        &self.directory
    }
}

#[derive(Debug, Deserialize)]
pub struct Hue {
    url: String,
    retry_ms: u64,
    max_delay_ms: u64,
    application_key: String,
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

#[cfg(test)]
pub struct AppConfigBuilder {
    config: AppConfig,
}

#[cfg(test)]
impl AppConfigBuilder {
    pub fn new() -> Self {
        AppConfigBuilder {
            config: AppConfig {
                core: Core { store_buffer_size: 1 },
                flows: Flows {
                    directory: "flows".to_string(),
                },
                hue: Hue {
                    url: "https://hue.url/".to_string(),
                    retry_ms: 100,
                    max_delay_ms: 200,
                    application_key: "key".to_string(),
                },
            },
        }
    }

    pub fn hue_url(mut self, url: String) -> Self {
        self.config.hue.url = url;
        self
    }

    pub fn build(self) -> AppConfig {
        self.config
    }
}
