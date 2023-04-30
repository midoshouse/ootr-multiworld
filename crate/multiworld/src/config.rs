use {
    std::{
        fs,
        path::PathBuf,
    },
    directories::ProjectDirs,
    once_cell::sync::Lazy,
    serde::{
        Deserialize,
        Serialize,
    },
    url::Url,
};

fn default_websocket_hostname() -> String { format!("mw.midos.house") }

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub log: bool,
    pub pj64_script_path: Option<PathBuf>,
    #[serde(default = "default_websocket_hostname")]
    pub websocket_hostname: String,
}

#[derive(Debug, thiserror::Error)]
pub enum SaveError {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error("failed to find project folder")]
    ProjectDirs,
}

impl Config {
    pub fn save(&self) -> Result<(), SaveError> {
        let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(SaveError::ProjectDirs)?;
        fs::create_dir_all(project_dirs.config_dir())?;
        fs::write(project_dirs.config_dir().join("config.json"), serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }

    pub fn websocket_url(&self) -> Result<Url, url::ParseError> {
        Url::parse(&format!("wss://{}/v{}", self.websocket_hostname, crate::version().major))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log: false,
            pj64_script_path: None,
            websocket_hostname: default_websocket_hostname(),
        }
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    if let Some(project_dirs) = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld") {
        if let Ok(config) = fs::read_to_string(project_dirs.config_dir().join("config.json")) {
            match serde_json::from_str::<Config>(&config) {
                Ok(config) => return config,
                #[cfg(debug_assertions)] Err(e) => eprintln!("{e:?}"),
                #[cfg(not(debug_assertions))] Err(_) => {} //TODO more visible error handling?
            }
        }
    }
    Config::default()
});
