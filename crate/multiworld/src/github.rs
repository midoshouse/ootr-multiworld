use {
    reqwest::{
        Client,
        StatusCode,
    },
    semver::Version,
    serde::Deserialize,
    url::Url,
};

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub assets: Vec<ReleaseAsset>,
    tag_name: String,
}

impl Release {
    pub fn version(&self) -> Result<Version, semver::Error> {
        self.tag_name[1..].parse()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: Url,
}

/// A GitHub repository. Provides API methods.
#[derive(Clone)]
pub struct Repo {
    /// The GitHub user or organization who owns this repo.
    user: String,
    /// The name of the repo.
    name: String,
}

impl Repo {
    pub fn new(user: impl ToString, name: impl ToString) -> Self {
        Self {
            user: user.to_string(),
            name: name.to_string(),
        }
    }

    pub async fn latest_release(&self, client: &Client) -> reqwest::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/latest", self.user, self.name))
            .send().await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no releases yet
        Ok(Some(
            response.error_for_status()?
                .json::<Release>().await?
        ))
    }

    pub fn latest_release_sync(&self, client: &reqwest::blocking::Client) -> reqwest::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/latest", self.user, self.name))
            .send()?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no releases yet
        Ok(Some(
            response.error_for_status()?
                .json::<Release>()?
        ))
    }

    pub async fn release_by_tag(&self, client: &Client, tag: &str) -> reqwest::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/tags/{tag}", self.user, self.name))
            .send().await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no release with this tag
        Ok(Some(
            response.error_for_status()?
                .json::<Release>().await?
        ))
    }
}
