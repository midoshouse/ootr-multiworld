use {
    std::{
        fs::File,
        num::NonZeroU8,
    },
    async_proto::Protocol,
    if_chain::if_chain,
    log_lock::*,
    ootr_utils::spoiler::HashIcon,
};
#[cfg(unix)] use xdg::BaseDirectories;
#[cfg(windows)] use directories::ProjectDirs;

const VERSION: u8 = 2;

#[derive(Protocol)]
pub(crate) struct PendingItem {
    pub(crate) hash: Option<[HashIcon; 5]>,
    pub(crate) key: u64,
    pub(crate) kind: u16,
    pub(crate) target_world: NonZeroU8,
}

#[derive(Default, Protocol)]
pub(crate) struct Data {
    pub(crate) pending_items_before_save: Vec<PendingItem>,
    pub(crate) pending_items_after_save: Vec<PendingItem>,
}

#[derive(Default, Clone)]
pub(crate) struct PersistentState(ArcRwLock<Data>);

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Io(#[from] std::io::Error),
    #[error(transparent)] Read(#[from] async_proto::ReadError),
    #[error(transparent)] Wheel(#[from] wheel::Error),
    #[error(transparent)] Write(#[from] async_proto::WriteError),
    #[cfg(windows)]
    #[error("failed to find project folder")]
    ProjectDirs,
}

impl PersistentState {
    pub(crate) fn blocking_load() -> Result<Self, Error> {
        let path = {
            #[cfg(unix)] {
                BaseDirectories::new().find_data_file("midos-house/multiworld-state.asyncproto")
            }
            #[cfg(windows)] {
                Some(ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::ProjectDirs)?.data_local_dir().join("state.asyncproto"))
            }
        };
        Ok(if_chain! {
            if let Some(path) = path;
            if path.exists();
            let mut file = File::open(path)?;
            if u8::read_sync(&mut file)? == VERSION;
            then {
                Self(ArcRwLock::new(Data::read_sync(&mut file)?))
            } else {
                Self(ArcRwLock::default())
            }
        })
    }

    pub(crate) async fn edit<T>(&self, f: impl FnOnce(&mut Data) -> T) -> Result<T, Error> {
        let output = lock!(@write state = self.0; f(&mut *state));
        let path = {
            #[cfg(unix)] {
                BaseDirectories::new().place_data_file("midos-house/multiworld-state.asyncproto")?
            }
            #[cfg(windows)] {
                let project_dirs = ProjectDirs::from("net", "Fenhl", "OoTR Multiworld").ok_or(Error::ProjectDirs)?;
                wheel::fs::create_dir_all(project_dirs.data_local_dir()).await?;
                project_dirs.data_local_dir().join("state.asyncproto")
            }
        };
        let mut file = wheel::fs::File::create(path).await?;
        VERSION.write(&mut file).await?;
        lock!(@read state = self.0; state.write(&mut file).await)?;
        Ok(output)
    }
}
