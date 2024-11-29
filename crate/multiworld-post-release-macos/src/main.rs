use {
    std::borrow::Cow,
    futures::io::AllowStdIo,
    itertools::Itertools as _,
    lazy_regex::regex_captures,
    semver::Version,
    sha2::{
        Digest as _,
        Sha256,
    },
    tokio::{
        io::{
            self,
            AsyncWriteExt as _,
        },
        process::Command,
    },
    tokio_util::compat::FuturesAsyncWriteCompatExt as _,
    wheel::{
        fs::{
            self,
            File,
        },
        traits::{
            AsyncCommandOutputExt as _,
            IoResultExt as _,
        },
    },
};

const TAP_STAGE: &str = "/opt/git/github.com/midoshouse/homebrew-tap/stage";
const CASK_PATH: &str = "/opt/git/github.com/midoshouse/homebrew-tap/stage/Casks/mhmw.rb";
const DMG_PATH: &str = "/opt/git/github.com/midoshouse/ootr-multiworld/main/assets/multiworld-gui.dmg";

#[derive(clap::Parser)]
struct Args {
    new_version: Version,
}

#[wheel::main]
async fn main(Args { new_version }: Args) -> wheel::Result {
    //TODO make sure stage is clean
    let cask = fs::read_to_string(CASK_PATH).await?;
    let mut lines = cask.lines().map(Cow::Borrowed).collect_vec();
    for line in &mut lines {
        if let Some((_, prefix, suffix)) = regex_captures!("^( *version \").+(\")$", line) {
            *line = Cow::Owned(format!("{prefix}{new_version}{suffix}"));
        } else if let Some((_, prefix, suffix)) = regex_captures!("^( *sha256 \").+(\")$", line) {
            let mut dmg = File::open(DMG_PATH).await?;
            let mut hasher = AllowStdIo::new(Sha256::default()).compat_write();
            io::copy(&mut dmg, &mut hasher).await.at(DMG_PATH)?;
            *line = Cow::Owned(format!("{prefix}{:02x}{suffix}", hasher.into_inner().into_inner().finalize().into_iter().format("")));
        }
    }
    {
        let mut file = File::create(CASK_PATH).await?;
        for line in lines {
            file.write_all(line.as_bytes()).await.at(CASK_PATH)?;
            file.write_all(b"\n").await.at(CASK_PATH)?;
        }
        file.flush().await.at(CASK_PATH)?;
    }
    Command::new("git").arg("add").arg("Casks/mhmw.rb").current_dir(TAP_STAGE).check("git add").await?;
    Command::new("git").arg("commit").arg("-m").arg("Update mhmw cask").current_dir(TAP_STAGE).check("git commit").await?;
    Command::new("git").arg("push").current_dir(TAP_STAGE).check("git push").await?;
    Ok(())
}
