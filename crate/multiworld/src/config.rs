use {
    std::{
        fs,
        path::PathBuf,
    },
    if_chain::if_chain,
    once_cell::sync::Lazy,
    serde::{
        Deserialize,
        Serialize,
    },
    url::Url,
};
#[cfg(unix)] use xdg::BaseDirectories;
#[cfg(windows)] use directories::ProjectDirs;

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
pub enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[cfg(unix)] #[error(transparent)] Xdg(#[from] xdg::BaseDirectoriesError),
    #[cfg(windows)]
    #[error("failed to find project folder")]
    ProjectDirs,
}

impl Config {
    fn load() -> Result<Self, Error> {
        let path = {
            #[cfg(unix)] {
                BaseDirectories::new()?.find_config_file("midos-house/multiworld.json")
            }
            #[cfg(windows)] {
                Some(ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::ProjectDirs)?.config_dir().join("config.json"))
            }
        };
        Ok(if_chain! {
            if let Some(path) = path;
            if path.exists();
            then {
                serde_json::from_str(&fs::read_to_string(path)?)?
            } else {
                Self::default()
            }
        })
    }

    pub fn save(&self) -> Result<(), Error> {
        let path = {
            #[cfg(unix)] {
                BaseDirectories::new()?.place_config_file("midos-house/multiworld.json")?
            }
            #[cfg(windows)] {
                let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::ProjectDirs)?;
                fs::create_dir_all(project_dirs.config_dir())?;
                project_dirs.config_dir().join("config.json")
            }
        };
        fs::write(path, serde_json::to_vec_pretty(self)?)?;
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

//TODO only use this where proper error reporting is infeasible
pub static CONFIG: Lazy<Config> = Lazy::new(|| {
    match Config::load() {
        Ok(config) => return config,
        #[cfg(debug_assertions)] Err(e) => eprintln!("{e:?}"),
        #[cfg(not(debug_assertions))] Err(_) => {}
    }
    Config::default()
});
