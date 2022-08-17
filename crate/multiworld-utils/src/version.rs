use {
    std::{
        iter,
        process::Stdio,
        str::FromStr as _,
    },
    graphql_client::GraphQLQuery,
    itertools::Itertools as _,
    semver::Version,
    tokio::process::Command,
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

pub async fn check_cli_version(package: &str, version: &Version) {
    let cli_output = String::from_utf8(Command::new("cargo").arg("run").arg(format!("--package={package}")).arg("--").arg("--version").stdout(Stdio::piped()).output().await.expect("failed to run CLI with --version").stdout).expect("CLI version output is invalid UTF-8");
    let (cli_name, cli_version) = cli_output.trim_end().split(' ').collect_tuple().expect("no space in CLI version output");
    assert_eq!(cli_name, package);
    assert_eq!(*version, cli_version.parse().expect("failed to parse CLI version"));
}

pub async fn version() -> Version {
    let version = Version::parse(env!("CARGO_PKG_VERSION")).expect("failed to parse current version");
    assert_eq!(version, multiworld::version());
    assert_eq!(version, multiworld_bizhawk::version());
    //assert_eq!(version, multiworld_csharp::version()); //TODO
    check_cli_version("multiworld-admin-cli", &version).await;
    check_cli_version("multiworld-installer", &version).await;
    check_cli_version("multiworld-pj64-gui", &version).await;
    check_cli_version("multiworld-updater", &version).await;
    check_cli_version("ootrmwd", &version).await;
    version
}
