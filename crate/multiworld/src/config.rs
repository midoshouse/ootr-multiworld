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
};

fn make_default_port() -> u16 { crate::PORT }

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    pub log: bool,
    pub pj64_script_path: Option<PathBuf>,
    #[serde(default = "make_default_port")]
    pub port: u16,
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log: false,
            pj64_script_path: None,
            port: crate::PORT,
        }
    }
}

pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    if let Some(project_dirs) = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld") {
        if let Ok(config) = fs::read_to_string(project_dirs.config_dir().join("config.json")) {
            if let Ok(config) = serde_json::from_str::<Config>(&config) {
                return config
            }
        }
    }
    Config::default()
});
