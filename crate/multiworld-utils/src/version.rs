use {
    std::{
        iter,
        str::FromStr as _,
    },
    graphql_client::GraphQLQuery,
    itertools::Itertools as _,
    semver::Version,
};

#[derive(Debug, thiserror::Error)]
pub enum BizHawkError {
    #[error(transparent)] ParseInt(#[from] std::num::ParseIntError),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error("no info returned in BizHawk version query response")]
    EmptyResponse,
    #[error("no BizHawk repo info returned")]
    MissingRepo,
    #[error("no releases in BizHawk GitHub repo")]
    NoReleases,
    #[error("the latest BizHawk GitHub release has no name")]
    UnnamedRelease,
}

#[cfg(windows)]
#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "../../assets/graphql/github-schema.graphql",
    query_path = "../../assets/graphql/github-bizhawk-version.graphql",
)]
struct BizHawkVersionQuery;

pub async fn bizhawk_latest(client: &reqwest::Client) -> Result<Version, BizHawkError> {
    let remote_version_string = client.post("https://api.github.com/graphql")
        .bearer_auth(include_str!("../../../assets/release-token"))
        .json(&BizHawkVersionQuery::build_query(biz_hawk_version_query::Variables {}))
        .send().await?
        .error_for_status()?
        .json::<graphql_client::Response<biz_hawk_version_query::ResponseData>>().await?
        .data.ok_or(BizHawkError::EmptyResponse)?
        .repository.ok_or(BizHawkError::MissingRepo)?
        .latest_release.ok_or(BizHawkError::NoReleases)?
        .name.ok_or(BizHawkError::UnnamedRelease)?;
    let (major, minor, patch) = remote_version_string.split('.').map(u64::from_str).chain(iter::repeat(Ok(0))).next_tuple().expect("iter::repeat produces an infinite iterator");
    Ok(Version::new(major?, minor?, patch?))
}

pub async fn version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse current version")
}
