use {
    std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
    },
    if_chain::if_chain,
    serde::{
        Deserialize,
        Serialize,
    },
    url::Url,
    crate::{
        frontend::Kind as Frontend,
        localization::Locale,
    },
};
#[cfg(unix)] use xdg::BaseDirectories;
#[cfg(windows)] use directories::ProjectDirs;

fn default_websocket_hostname() -> String { format!("mw.midos.house") }

#[derive(Clone, Deserialize, Serialize)]
pub struct Config {
    pub default_frontend: Option<Frontend>,
    #[serde(default)]
    pub log: bool,
    #[serde(default)]
    pub login_tokens: BTreeMap<crate::IdentityProvider, String>,
    #[serde(default)]
    pub refresh_tokens: BTreeMap<crate::IdentityProvider, String>,
    pub pj64_script_path: Option<PathBuf>,
    pub locale: Option<Locale>,
    #[serde(default = "default_websocket_hostname")]
    pub websocket_hostname: String,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Json(#[from] serde_json::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[cfg(unix)] #[error(transparent)] Xdg(#[from] xdg::BaseDirectoriesError),
    #[cfg(windows)]
    #[error("failed to find project folder")]
    ProjectDirs,
}

impl Config {
    pub fn blocking_load() -> Result<Self, Error> {
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

    pub async fn load() -> Result<Self, Error> {
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
            if wheel::fs::exists(&path).await?;
            then {
                wheel::fs::read_json(path).await?
            } else {
                Self::default()
            }
        })
    }

    pub async fn save(&self) -> Result<(), Error> {
        let path = {
            #[cfg(unix)] {
                BaseDirectories::new()?.place_config_file("midos-house/multiworld.json")?
            }
            #[cfg(windows)] {
                let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::ProjectDirs)?;
                wheel::fs::create_dir_all(project_dirs.config_dir()).await?;
                project_dirs.config_dir().join("config.json")
            }
        };
        wheel::fs::write_json(path, self).await?;
        Ok(())
    }

    pub fn websocket_url(&self) -> Result<Url, url::ParseError> {
        Url::parse(&format!("wss://{}/v{}", self.websocket_hostname, crate::version().major))
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_frontend: None,
            log: false,
            login_tokens: BTreeMap::default(),
            refresh_tokens: BTreeMap::default(),
            pj64_script_path: None,
            websocket_hostname: default_websocket_hostname(),
            locale: None,
        }
    }
}
