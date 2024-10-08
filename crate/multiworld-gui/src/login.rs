use {
    std::{
        any::TypeId,
        fmt,
        hash::Hash as _,
        marker::PhantomData,
        pin::Pin,
        sync::Arc,
    },
    futures::{
        future,
        stream::{
            self,
            Stream,
            StreamExt as _,
            TryStreamExt as _,
        },
    },
    iced::advanced::subscription::{
        EventStream,
        Recipe,
    },
    oauth2::{
        AuthUrl,
        AuthorizationCode,
        ClientId,
        CsrfToken,
        PkceCodeChallenge,
        PkceCodeVerifier,
        RedirectUrl,
        Scope,
        TokenResponse as _,
        TokenUrl,
        basic::BasicClient,
        reqwest::async_http_client,
    },
    rocket::{
        Rocket,
        State,
        request::Request,
        response::{
            Responder,
            content::RawHtml,
        },
    },
    rocket_util::{
        Doctype,
        ToHtml,
        html,
    },
    tokio::{
        sync::mpsc,
        task::JoinHandle,
    },
    tokio_stream::wrappers::ReceiverStream,
    crate::Message,
};
pub(crate) use multiworld::IdentityProvider as Provider;

const PORT: u16 = 24819;

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)] Rocket(#[from] rocket::Error),
    #[error(transparent)] Url(#[from] url::ParseError),
}

pub(crate) fn oauth_client(provider: Provider) -> Result<BasicClient, url::ParseError> {
    Ok(match provider {
        Provider::RaceTime => BasicClient::new(
            ClientId::new(env!("CLIENT_ID_RACETIME").to_owned()),
            None,
            AuthUrl::new(format!("https://racetime.gg/o/authorize"))?,
            Some(TokenUrl::new(format!("https://racetime.gg/o/token"))?),
        ).set_redirect_uri(RedirectUrl::new(format!("http://localhost:{PORT}/auth/racetime"))?),
        Provider::Discord => BasicClient::new(
            ClientId::new(env!("CLIENT_ID_DISCORD").to_owned()),
            None,
            AuthUrl::new(format!("https://discord.com/oauth2/authorize"))?,
            Some(TokenUrl::new(format!("https://discord.com/api/oauth2/token"))?),
        ).set_redirect_uri(RedirectUrl::new(format!("http://localhost:{PORT}/auth/discord"))?),
    })
}

pub(crate) struct Subscription(pub(crate) Provider);

impl Recipe for Subscription {
    type Output = Message;

    fn hash(&self, state: &mut iced::advanced::subscription::Hasher) {
        TypeId::of::<Self>().hash(state);
        self.0.hash(state);
    }

    fn stream(self: Box<Self>, _: EventStream) -> Pin<Box<dyn Stream<Item = Message> + Send>> {
        struct RocketDropStream<S: Stream> {
            rocket: JoinHandle<()>,
            stream: S,
        }

        impl<S: Stream> Stream for RocketDropStream<S> {
            type Item = S::Item;

            fn poll_next(self: Pin<&mut Self>, cx: &mut futures::task::Context<'_>) -> futures::task::Poll<Option<S::Item>> {
                unsafe { self.map_unchecked_mut(|s| &mut s.stream) }.poll_next(cx)
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.stream.size_hint()
            }
        }

        impl<S: Stream> Drop for RocketDropStream<S> {
            fn drop(&mut self) {
                self.rocket.abort();
            }
        }

        stream::once(async move {
            let provider = self.0;
            let oauth_client = oauth_client(provider)?;
            let (pkce_challenge, pkce_verifier) = match provider {
                Provider::RaceTime => (None, None),
                Provider::Discord => {
                    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
                    (Some(pkce_challenge), Some(pkce_verifier))
                }
            };
            let (auth_url, csrf_token) = {
                let mut auth_request = oauth_client.authorize_url(CsrfToken::new_random);
                auth_request = auth_request.add_scope(Scope::new(match provider {
                    Provider::RaceTime => format!("read"),
                    Provider::Discord => format!("identify"),
                }));
                if let Some(pkce_challenge) = pkce_challenge {
                    auth_request = auth_request.set_pkce_challenge(pkce_challenge);
                }
                auth_request.url()
            };
            let (token_tx, token_rx) = mpsc::channel::<(String, Option<String>)>(1);
            let mut rocket = rocket::custom(rocket::Config {
                log_level: rocket::config::LogLevel::Critical,
                port: PORT,
                shutdown: rocket::config::Shutdown {
                    grace: 30,
                    mercy: 30,
                    ..rocket::config::Shutdown::default()
                },
                ..rocket::Config::default()
            }).mount("/", match provider {
                Provider::RaceTime => rocket::routes![racetime_callback],
                Provider::Discord => rocket::routes![discord_callback],
            })
            .register("/", rocket::catchers![
                not_found,
                internal_server_error,
            ])
            .manage(oauth_client)
            .manage(csrf_token)
            .manage(token_tx);
            if let Some(pkce_verifier) = pkce_verifier {
                rocket = rocket.manage(pkce_verifier);
            }
            let rocket = rocket.ignite().await?;
            Ok::<_, Error>(RocketDropStream {
                rocket: tokio::spawn(async move {
                    let Rocket { .. } = rocket.launch().await.expect("error in OAuth callback web server");
                }),
                stream: stream::once(future::ready(Message::OpenLoginPage(auth_url)))
                    .chain(ReceiverStream::new(token_rx).map(move |(bearer_token, refresh_token)| Message::LoginTokens { provider, bearer_token, refresh_token }))
                    .map(Ok),
            })
        })
        .try_flatten()
        .map(|res| match res {
            Ok(msg) => msg,
            Err(e) => Message::LoginError(Arc::new(e)),
        })
        .boxed()
    }
}

fn page(title: &str, content: impl ToHtml) -> RawHtml<String> {
    html! {
        : Doctype;
        html {
            head {
                meta(charset = "utf-8");
                title : title;
                meta(name = "viewport", content = "width=device-width, initial-scale=1, shrink-to-fit=no");
                link(rel = "icon", sizes = "1024x1024", type = "image/png", href = "https://midos.house/static/mw.png");
                link(rel = "stylesheet", href = "https://midos.house/static/common.css");
            }
            body {
                div {
                    nav {
                        div(class = "logo") {
                            img(class = "mw-logo", src = "https://midos.house/static/mw.png");
                        }
                        h1 : "Mido's House Multiworld";
                    }
                    main {
                        : content;
                    }
                }
                footer {
                    p {
                        a(href = "https://github.com/midoshouse/ootr-multiworld") : "source code";
                    }
                    p : "Special thanks to Maplestar for the chest icon used in the logo!";
                }
            }
        }
    }
}

#[rocket::catch(404)]
fn not_found() -> RawHtml<String> {
    page("Not Found — Mido's House Multiworld", html! {
        h1 : "Error 404: Not Found";
        p : "There is no page at this address.";
        h2 : "Support";
        ul {
            li {
                : "Ask in #setup-support on the OoT Randomizer Discord (";
                a(href = "https://discord.gg/BGRrKKn") : "invite link";
                : " • ";
                a(href = "https://discord.com/channels/274180765816848384/476723801032491008") : "direct channel link";
                : "). Feel free to ping @fenhl.";
            }
            li : "Ask in #general on the OoTR MW Tournament Discord.";
            li {
                : "Or ";
                a(href = "https://github.com/midoshouse/ootr-multiworld/issues/new") : "open an issue";
                : ".";
            }
        }
    })
}

#[rocket::catch(500)]
fn internal_server_error() -> RawHtml<String> {
    page("Internal Server Error — Mido's House Multiworld", html! {
        h1 : "Error 500: Internal Server Error";
        p : "An error occurred while trying to sign in.";
        //TODO show error (global mutex holding last error?)
        h2 : "Support";
        p : "This is a bug in Mido's House Multiworld. Please report it:";
        ul {
            li {
                a(href = "https://github.com/midoshouse/ootr-multiworld/issues/new") : "Open a GitHub issue";
                : ",";
            }
            li {
                : "Or post in #setup-support on the OoT Randomizer Discord (";
                a(href = "https://discord.gg/BGRrKKn") : "invite link";
                : " • ";
                a(href = "https://discord.com/channels/274180765816848384/476723801032491008") : "direct channel link";
                : "). Please ping @fenhl in your message.";
            }
            li : "Or ask in #general on the OoTR MW Tournament Discord.";
        }
    })
}

#[derive(Debug)] enum RaceTime {}
#[derive(Debug)] enum Discord {}

trait ProviderTrait: fmt::Debug {
    const NAME: &'static str;
}

impl ProviderTrait for Discord {
    const NAME: &'static str = "Discord";
}

impl ProviderTrait for RaceTime {
    const NAME: &'static str = "racetime.gg";
}

#[derive(Debug, thiserror::Error)]
enum CallbackError<P: ProviderTrait> {
    #[error(transparent)] RequestToken(#[from] oauth2::basic::BasicRequestTokenError<oauth2::reqwest::HttpClientError>),
    #[error("invalid CSRF token")]
    CsrfMismatch,
    #[error("failed to send bearer token to multiworld app")]
    Send,
    #[allow(unused)] #[error("unreachable")]
    _Phantom(PhantomData<P>),
}

impl<'r, P: ProviderTrait> Responder<'r, 'static> for CallbackError<P> {
    fn respond_to(self, request: &'r Request<'_>) -> rocket::response::Result<'static> {
        page("Login failed — Mido's House Multiworld", html! {
            h1 : "Login failed";
            p {
                : "Sorry, it seems that there was an error trying to sign in with ";
                : P::NAME;
                : ":";
            }
            p : self.to_string();
            p {
                : "Debug info: ";
                code : format!("{self:?}");
            }
        }).respond_to(request)
    }
}

#[rocket::get("/auth/discord?<code>&<state>")]
async fn discord_callback(oauth_client: &State<BasicClient>, csrf_token: &State<CsrfToken>, pkce_verifier: &State<PkceCodeVerifier>, shutdown: rocket::Shutdown, sender: &State<mpsc::Sender<(String, Option<String>)>>, code: String, state: String) -> Result<RawHtml<String>, CallbackError<Discord>> {
    if state != *csrf_token.secret() {
        return Err(CallbackError::CsrfMismatch)
    }
    let tokens = oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.secret().clone())) // need to extract and rebuild a `PkceCodeVerifier` because it doesn't implement `Clone`
        .request_async(async_http_client).await?;
    sender.send((tokens.access_token().secret().clone(), tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()))).await.map_err(|_| CallbackError::Send)?;
    shutdown.notify();
    Ok(page("Login successful — Mido's House Multiworld", html! {
        h1 : "Discord login successful";
        p : "You can now close this tab and continue in the multiworld app.";
    }))
}

#[rocket::get("/auth/racetime?<code>&<state>")]
async fn racetime_callback(oauth_client: &State<BasicClient>, csrf_token: &State<CsrfToken>, shutdown: rocket::Shutdown, sender: &State<mpsc::Sender<(String, Option<String>)>>, code: String, state: String) -> Result<RawHtml<String>, CallbackError<RaceTime>> {
    if state != *csrf_token.secret() {
        return Err(CallbackError::CsrfMismatch)
    }
    let tokens = oauth_client
        .exchange_code(AuthorizationCode::new(code))
        .request_async(async_http_client).await?;
    sender.send((tokens.access_token().secret().clone(), tokens.refresh_token().map(|refresh_token| refresh_token.secret().clone()))).await.map_err(|_| CallbackError::Send)?;
    shutdown.notify();
    Ok(page("Login successful — Mido's House Multiworld", html! {
        h1 : "racetime.gg login successful";
        p : "You can now close this tab and continue in the multiworld app.";
    }))
}
