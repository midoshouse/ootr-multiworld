use {
    std::future::Future,
    reqwest::{
        Body,
        Client,
        StatusCode,
    },
    semver::Version,
    serde::Deserialize,
    serde_json::json,
    url::Url,
    wheel::traits::{
        RequestBuilderExt as _,
        ReqwestResponseExt as _,
    },
};

#[cfg(feature = "github-app-auth")]
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)] Auth(#[from] github_app_auth::AuthError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error(transparent)] Wheel(#[from] wheel::Error),
}

#[cfg(feature = "github-app-auth")]
#[derive(Debug, Deserialize)]
pub struct Issue {
    number: u64,
    pub labels: Vec<Label>,
}

#[cfg(feature = "github-app-auth")]
impl Issue {
    pub async fn set_labels(&self, client: &Client, token: &mut github_app_auth::InstallationAccessToken, repo: &Repo, labels: &[String]) -> Result<(), Error> {
        client.patch(&format!("https://api.github.com/repos/{}/{}/issues/{}", repo.user, repo.name, self.number))
            .headers(token.header().await?)
            .json(&json!({
                "labels": labels,
            }))
            .send_github(false).await?
            .detailed_error_for_status().await?;
        Ok(())
    }
}

#[cfg(feature = "github-app-auth")]
#[derive(Debug, Deserialize)]
pub struct Label {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Release {
    pub assets: Vec<ReleaseAsset>,
    id: u64,
    pub tag_name: String,
    upload_url: String,
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

    #[cfg(feature = "github-app-auth")]
    pub async fn issues_with_label(&self, client: &Client, token: &mut github_app_auth::InstallationAccessToken, label: &str) -> Result<Vec<Issue>, Error> {
        Ok(client.get(&format!("https://api.github.com/repos/{}/{}/issues", self.user, self.name))
            .headers(token.header().await?)
            .query(&[
                ("state", "all"),
                ("labels", label),
            ])
            .send_github(false).await?
            .detailed_error_for_status().await?
            .json_with_text_in_error().await?)
    }

    pub async fn latest_release(&self, client: &Client) -> wheel::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/latest", self.user, self.name))
            .send_github(false).await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no releases yet
        Ok(Some(
            response.detailed_error_for_status().await?
                .json_with_text_in_error::<Release>().await?
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

    pub async fn release_by_tag(&self, client: &Client, tag: &str) -> wheel::Result<Option<Release>> {
        let response = client.get(&format!("https://api.github.com/repos/{}/{}/releases/tags/{tag}", self.user, self.name))
            .send_github(false).await?;
        if response.status() == StatusCode::NOT_FOUND { return Ok(None) } // no release with this tag
        Ok(Some(
            response.detailed_error_for_status().await?
                .json_with_text_in_error::<Release>().await?
        ))
    }

    /// Creates a draft release, which can be published using `Repo::publish_release`.
    pub async fn create_release(&self, client: &Client, name: String, tag_name: String, body: String) -> wheel::Result<Release> {
        Ok(
            client.post(&format!("https://api.github.com/repos/{}/{}/releases", self.user, self.name))
                .json(&json!({
                    "body": body,
                    "draft": true,
                    "name": name,
                    "tag_name": tag_name,
                }))
                .send_github(false).await?
                .detailed_error_for_status().await?
                .json_with_text_in_error::<Release>().await?
        )
    }

    pub async fn publish_release(&self, client: &Client, release: Release) -> wheel::Result<Release> {
        Ok(
            client.patch(&format!("https://api.github.com/repos/{}/{}/releases/{}", self.user, self.name, release.id))
                .json(&json!({"draft": false}))
                .send_github(false).await?
                .detailed_error_for_status().await?
                .json_with_text_in_error::<Release>().await?
        )
    }

    pub fn release_attach<'a>(&self, client: &'a Client, release: &'a Release, name: &'a str, content_type: &'static str, body: impl Into<Body> + 'a) -> impl Future<Output = wheel::Result<()>> + 'a {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::CONTENT_TYPE, reqwest::header::HeaderValue::from_static(content_type));
        async move {
            client.post(&release.upload_url.replace("{?name,label}", ""))
                .query(&[("name", name)])
                .headers(headers)
                .body(body)
                .send_github(false).await?
                .detailed_error_for_status().await?;
            Ok(())
        }
    }
}
