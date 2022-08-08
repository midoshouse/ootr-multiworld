#![deny(rust_2018_idioms, unused, unused_crate_dependencies, unused_import_braces, unused_lifetimes, unused_qualifications, warnings)]
#![forbid(unsafe_code)]

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use {
    std::{
        path::PathBuf,
        sync::Arc,
        time::Duration,
    },
    futures::{
        future::Future,
        stream::TryStreamExt as _,
    },
    iced::{
        Command,
        Length,
        Settings,
        alignment,
        pure::{
            Application,
            Element,
            widget::*,
        },
        window,
    },
    itertools::Itertools as _,
    open::that as open,
    tokio::{
        fs::File,
        io::AsyncWriteExt as _,
        time::sleep,
    },
    multiworld::github::{
        ReleaseAsset,
        Repo,
    },
};

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error(transparent)] Io(#[from] tokio::io::Error),
    #[error(transparent)] Reqwest(#[from] reqwest::Error),
    #[error("clone of unexpected message kind")]
    Cloned,
    #[error("latest release does not have a download for this platform")]
    MissingAsset,
    #[error("there are no released versions")]
    NoReleases,
}

#[derive(Debug)]
enum Message {
    Error(Arc<Error>),
    ReleaseAsset(reqwest::Client, ReleaseAsset),
    WaitedExit(reqwest::Client, ReleaseAsset),
    Response(reqwest::Response),
    Downloaded(File),
    WaitedDownload,
    Done,
    DiscordInvite,
    DiscordChannel,
    NewIssue,
    Cloned,
}

impl Clone for Message {
    fn clone(&self) -> Self {
        match self {
            Self::Error(e) => Self::Error(e.clone()),
            Self::DiscordInvite => Self::DiscordInvite,
            Self::DiscordChannel => Self::DiscordChannel,
            Self::NewIssue => Self::NewIssue,
            _ => Self::Cloned,
        }
    }
}

fn cmd(future: impl Future<Output = Result<Message, Error>> + Send + 'static) -> Command<Message> {
    Command::single(iced_native::command::Action::Future(Box::pin(async move {
        match future.await {
            Ok(msg) => msg,
            Err(e) => Message::Error(Arc::new(e.into())),
        }
    })))
}

enum State {
    Init,
    WaitExit,
    Download,
    Replace,
    WaitDownload,
    Launch,
    Done,
    Error(Arc<Error>),
}

struct App {
    path: PathBuf,
    state: State,
}

impl Application for App {
    type Executor = iced::executor::Default;
    type Message = Message;
    type Flags = Args;

    fn new(Args { package, path }: Args) -> (Self, Command<Message>) {
        (App {
            state: State::Init,
            path,
        }, cmd(async move {
            let http_client = reqwest::Client::builder()
                .user_agent(concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION")))
                .use_rustls_tls()
                .https_only(true)
                .http2_prior_knowledge()
                .build()?;
            let release = Repo::new("midoshouse", "ootr-multiworld").latest_release(&http_client).await?.ok_or(Error::NoReleases)?;
            let (asset,) = release.assets.into_iter()
                .filter(|asset| asset.name == package.asset_name())
                .collect_tuple().ok_or(Error::MissingAsset)?;
            Ok(Message::ReleaseAsset(http_client, asset))
        }))
    }

    fn title(&self) -> String { format!("updating Mido's House Multiworld…") }

    fn update(&mut self, msg: Message) -> Command<Message> {
        match msg {
            Message::Error(e) => self.state = State::Error(e),
            Message::ReleaseAsset(http_client, asset) => {
                self.state = State::WaitExit;
                return cmd(async {
                    sleep(Duration::from_secs(1)).await;
                    Ok(Message::WaitedExit(http_client, asset))
                })
            }
            Message::WaitedExit(http_client, asset) => {
                self.state = State::Download;
                return cmd(async move {
                    Ok(Message::Response(http_client.get(asset.browser_download_url).send().await?.error_for_status()?))
                })
            }
            Message::Response(response) => {
                self.state = State::Replace;
                let path = self.path.clone();
                return cmd(async move {
                    let mut data = response.bytes_stream();
                    let mut exe_file = File::create(path).await?;
                    while let Some(chunk) = data.try_next().await? {
                        exe_file.write_all(chunk.as_ref()).await?;
                    }
                    Ok(Message::Downloaded(exe_file))
                })
            }
            Message::Downloaded(exe_file) => {
                self.state = State::WaitDownload;
                return cmd(async move {
                    exe_file.sync_all().await?;
                    Ok(Message::WaitedDownload)
                })
            }
            Message::WaitedDownload => {
                self.state = State::Launch;
                let path = self.path.clone();
                return cmd(async move {
                    std::process::Command::new(path).spawn()?;
                    Ok(Message::Done)
                })
            }
            Message::Done => self.state = State::Done,
            Message::DiscordInvite => if let Err(e) = open("https://discord.gg/BGRrKKn") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::DiscordChannel => if let Err(e) = open("https://discord.com/channels/274180765816848384/476723801032491008") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::NewIssue => if let Err(e) = open("https://github.com/midoshouse/ootr-multiworld/issues/new") {
                self.state = State::Error(Arc::new(e.into()));
            },
            Message::Cloned => self.state = State::Error(Arc::new(Error::Cloned)),
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Message> {
        match self.state {
            State::Init => Text::new("Checking latest release…").into(),
            State::WaitExit => Text::new("Waiting to make sure the old version has exited…").into(),
            State::Download => Text::new("Starting download…").into(),
            State::Replace => Text::new("Downloading update…").into(),
            State::WaitDownload => Text::new("Finishing download…").into(),
            State::Launch => Text::new("Starting new version…").into(),
            State::Done => Text::new("Closing updater…").into(),
            State::Error(ref e) => Column::new()
                .push(Text::new("Error").size(24).width(Length::Fill).horizontal_alignment(alignment::Horizontal::Center))
                .push(Text::new(e.to_string()))
                .push(Text::new(format!("debug info: {e:?}")))
                .push(Text::new("Support").size(24).width(Length::Fill).horizontal_alignment(alignment::Horizontal::Center))
                .push(Text::new("• Ask in #setup-support on the OoT Randomizer Discord. Feel free to ping @Fenhl#4813."))
                .push(Row::new()
                    .push(Button::new(Text::new("invite link")).on_press(Message::DiscordInvite))
                    .push(Button::new(Text::new("direct channel link")).on_press(Message::DiscordChannel))
                )
                .push(Row::new()
                    .push(Text::new("• Or "))
                    .push(Button::new(Text::new("open an issue")).on_press(Message::NewIssue))
                )
                .into(),
        }
    }

    fn should_exit(&self) -> bool {
        matches!(self.state, State::Done)
    }
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Package {
    Pj64,
}

impl Package {
    fn asset_name(&self) -> &'static str {
        match self {
            Self::Pj64 => "multiworld-pj64.exe",
        }
    }
}

#[derive(clap::Parser)]
#[clap(version)]
struct Args {
    #[clap(value_enum)]
    package: Package,
    #[clap(parse(from_os_str))]
    path: PathBuf,
}

#[wheel::main]
fn main(args: Args) -> iced::Result {
    App::run(Settings {
        window: window::Settings {
            size: (320, 240),
            ..window::Settings::default()
        },
        ..Settings::with_flags(args)
    })
}
