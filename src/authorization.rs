use std::{future::Future, pin::Pin, sync::Arc};

use axum::{
    body::Body,
    extract::State,
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{ErrorResponse, Response},
};
use axum_extra::{
    headers::{self, authorization::Bearer},
    TypedHeader,
};
use futures::{channel::oneshot, stream::FuturesUnordered, StreamExt};
use tokio::{select, sync::mpsc, task::JoinHandle};

pub struct Authorization {
    send: mpsc::Sender<AuthorizationMsg>,
    handle: JoinHandle<()>,
}

impl Authorization {
    pub fn new(authorization_url: String) -> Self {
        let client = HttpAuthorizationClient::new(authorization_url);
        Self::with_client(client)
    }

    pub fn with_client(client: impl AuthorizationClient) -> Self {
        let (send, recv) = mpsc::channel(1);
        let mut actor = AuthorizationActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Authorization { send, handle }
    }

    pub fn api(&self) -> AuthorizationApi {
        AuthorizationApi::new(self.send.clone())
    }

    /// Authorization is going to shutdown, as soon as the last instance of [`AuthorizationApi`] is
    /// dropped.
    pub async fn wait_for_shutdown(self) {
        drop(self.send);
        self.handle.await.unwrap();
    }
}

struct AuthorizationActor<C: AuthorizationClient> {
    recv: mpsc::Receiver<AuthorizationMsg>,
    running_authorizations: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
    client: Arc<C>,
}

impl<C: AuthorizationClient> AuthorizationActor<C> {
    fn new(client: C, recv: mpsc::Receiver<AuthorizationMsg>) -> Self {
        Self {
            recv,
            running_authorizations: FuturesUnordered::new(),
            client: Arc::new(client),
        }
    }
    async fn run(&mut self) {
        loop {
            select! {
                msg = self.recv.recv() => match msg {
                    Some(msg) =>  self.act(msg),
                    None => break
                },
                () = self.running_authorizations.select_next_some(), if !self.running_authorizations.is_empty() => {}
            }
        }
    }
    fn act(&mut self, msg: AuthorizationMsg) {
        let client = self.client.clone();
        let fut = async move {
            msg.act(client.as_ref()).await;
        };
        self.running_authorizations.push(Box::pin(fut));
    }
}

#[derive(Clone)]
pub struct AuthorizationApi {
    send: mpsc::Sender<AuthorizationMsg>,
}

impl AuthorizationApi {
    pub fn new(send: mpsc::Sender<AuthorizationMsg>) -> Self {
        Self { send }
    }

    pub async fn check_permission(&self, api_token: String) -> anyhow::Result<bool> {
        let (send, recv) = oneshot::channel();
        let msg = AuthorizationMsg::Auth { api_token, send };
        self.send
            .send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

pub enum AuthorizationMsg {
    Auth {
        api_token: String,
        send: oneshot::Sender<anyhow::Result<bool>>,
    },
}

impl AuthorizationMsg {
    async fn act(self, client: &impl AuthorizationClient) {
        match self {
            AuthorizationMsg::Auth { api_token, send } => {
                let result = client.token_valid(api_token).await;
                drop(send.send(result));
            }
        };
    }
}

pub async fn authorization_middleware(
    State(authorization_api): State<AuthorizationApi>,
    bearer: Option<TypedHeader<headers::Authorization<Bearer>>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ErrorResponse> {
    if let Some(bearer) = bearer {
        if let Ok(allowed) = authorization_api
            .check_permission(bearer.token().to_owned())
            .await
        {
            if !allowed {
                return Err(ErrorResponse::from((
                    StatusCode::FORBIDDEN,
                    "Bearer token invalid".to_owned(),
                )));
            }
        } else {
            return Err(ErrorResponse::from((
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to check permission".to_owned(),
            )));
        }
    } else {
        return Err(ErrorResponse::from((
            StatusCode::BAD_REQUEST,
            "Bearer token expected".to_owned(),
        )));
    }
    let response = next.run(request).await;
    Ok(response)
}

pub trait AuthorizationClient: Send + Sync + 'static {
    fn token_valid(&self, api_token: String) -> impl Future<Output = anyhow::Result<bool>> + Send;
}

struct HttpAuthorizationClient {
    url: String,
    client: reqwest::Client,
}

impl HttpAuthorizationClient {
    pub fn new(url: String) -> Self {
        Self {
            url,
            client: reqwest::Client::new(),
        }
    }
}

impl AuthorizationClient for HttpAuthorizationClient {
    async fn token_valid(&self, api_token: String) -> anyhow::Result<bool> {
        let url = format!("{}/check_privileges", self.url);
        let body: Vec<String> = vec![];
        let mut headers = HeaderMap::new();
        headers.insert(
            "Authorization",
            format!("Bearer {api_token}").parse().unwrap(),
        );
        let response = self
            .client
            .post(url)
            .json(&body)
            .headers(headers)
            .send()
            .await?;
        Ok(response.status().is_success())
    }
}

#[cfg(test)]
pub mod tests {

    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;
    use crate::tests::{api_token, inference_url};

    /// An authorization double, loaded up with predefined answers.
    pub struct StubAuthorization {
        send: mpsc::Sender<AuthorizationMsg>,
    }

    impl StubAuthorization {
        pub fn new(mut handle: impl FnMut(AuthorizationMsg) + Send + 'static) -> Self {
            let (send, mut recv) = mpsc::channel(1);
            tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    handle(msg);
                }
            });
            Self { send }
        }

        pub fn api(&self) -> AuthorizationApi {
            AuthorizationApi::new(self.send.clone())
        }
    }

    #[tokio::test]
    async fn true_for_valid_permissions() {
        // Given a client that is configured against the inference api
        let url = inference_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check a valid api token
        let result = client.token_valid(api_token().to_owned()).await;

        // Then the result is true
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn false_for_invalid_token() {
        // Given a client that is configured against the inference api
        let url = inference_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check an invalid api token
        let result = client.token_valid("invalid".to_owned()).await;

        // Then the result is false
        assert!(!result.unwrap());
    }

    struct StubAuthorizationClient;

    impl AuthorizationClient for StubAuthorizationClient {
        async fn token_valid(&self, _api_token: String) -> anyhow::Result<bool> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn auth_api_answers_messages() {
        // Given a stub authorization client
        let client = StubAuthorizationClient;
        let api = Authorization::with_client(client).api();

        // When checking permissions for a token
        let result = timeout(
            Duration::from_millis(10),
            api.check_permission("valid".to_owned()),
        )
        .await;

        // Then the message get's answered within 10ms
        assert!(result.unwrap().unwrap());
    }
}
