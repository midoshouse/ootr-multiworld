use {
    reqwest::{
        Client,
        StatusCode,
    },
    serde::Deserialize,
    url::Url,
};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Release {
    pub(crate) assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ReleaseAsset {
    pub(crate) name: String,
    pub(crate) browser_download_url: Url,
}

/// A GitHub repository. Provides API methods.
#[derive(Clone)]
pub(crate) struct Repo {
    /// The GitHub user or organization who owns this repo.
    user: String,
    /// The name of the repo.
    name: String,
}

impl Repo {
    pub(crate) fn new(user: impl ToString, name: impl ToString) -> Self {
        Self {
            user: user.to_string(),
            name: name.to_string(),
        }
    }

    pub(crate) async fn latest_release(&self, client: &Client) -> reqwest::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/latest", self.user, self.name))
            .send().await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no releases yet
        Ok(Some(
            response.error_for_status()?
                .json::<Release>().await?
        ))
    }
}
