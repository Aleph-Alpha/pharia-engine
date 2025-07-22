use std::{future::Future, pin::Pin, sync::Arc};

use axum::{extract::FromRef, http::StatusCode};
use futures::{StreamExt, channel::oneshot, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;
use tokio::{select, sync::mpsc, task::JoinHandle};
use tracing::error;

use crate::{http::HttpClient, logging::TracingContext};

/// The authentication provided by incoming requests.
/// When operating inside `PhariaAI`, users are required to provide a valid `PhariaAI` token. This
/// token is then used to authenticate all outgoing request, e.g. against the Aleph Alpha inference
/// or the DocumentIndex.
/// If the Kernel is being operated outside of `PhariaAI`, authentication for incoming requests is
/// optional. Skills can still execute inference requests against OpenAI-compatible inference
/// backends, if an api-token has been configured in the Kernel configuration. This e.g. allows
/// developers to run the Kernel locally.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authentication(Option<String>);

impl Authentication {
    pub fn new(token: impl Into<String>) -> Self {
        Self(Some(token.into()))
    }

    pub fn into_maybe_string(self) -> Option<String> {
        self.0
    }
}

pub struct Authorization {
    send: mpsc::Sender<AuthorizationMsg>,
    handle: JoinHandle<()>,
}

impl Authorization {
    pub fn new(authorization_url: Option<&str>) -> Self {
        if let Some(url) = authorization_url {
            Self::with_client(HttpAuthorizationClient::new(url))
        } else {
            Self::with_client(AlwaysValidClient)
        }
    }

    pub fn with_client(client: impl AuthorizationClient) -> Self {
        let (send, recv) = mpsc::channel(1);
        let mut actor = AuthorizationActor::new(client, recv);
        let handle = tokio::spawn(async move { actor.run().await });
        Authorization { send, handle }
    }

    pub fn api(&self) -> mpsc::Sender<AuthorizationMsg> {
        self.send.clone()
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
        self.running_authorizations.push(Box::pin(async move {
            msg.act(client.as_ref()).await;
        }));
    }
}

pub trait AuthorizationApi {
    fn check_permission(
        &self,
        auth: Authentication,
        context: TracingContext,
    ) -> impl Future<Output = Result<bool, AuthorizationClientError>> + Send;
}

impl AuthorizationApi for mpsc::Sender<AuthorizationMsg> {
    async fn check_permission(
        &self,
        auth: Authentication,
        context: TracingContext,
    ) -> Result<bool, AuthorizationClientError> {
        let (send, recv) = oneshot::channel();
        let msg = AuthorizationMsg::Auth {
            auth,
            context,
            send,
        };
        self.send(msg)
            .await
            .expect("all api handlers must be shutdown before actors");
        recv.await
            .expect("sender must be alive when awaiting for answers")
    }
}

pub enum AuthorizationMsg {
    Auth {
        auth: Authentication,
        context: TracingContext,
        send: oneshot::Sender<Result<bool, AuthorizationClientError>>,
    },
}

impl AuthorizationMsg {
    async fn act(self, client: &impl AuthorizationClient) {
        match self {
            AuthorizationMsg::Auth {
                auth,
                context,
                send,
            } => {
                let result = client.token_valid(auth, context).await;
                drop(send.send(result));
            }
        }
    }
}

pub trait AuthorizationClient: Send + Sync + 'static {
    fn token_valid(
        &self,
        auth: Authentication,
        context: TracingContext,
    ) -> impl Future<Output = Result<bool, AuthorizationClientError>> + Send;
}

struct AlwaysValidClient;

impl AuthorizationClient for AlwaysValidClient {
    async fn token_valid(
        &self,
        _auth: Authentication,
        _context: TracingContext,
    ) -> Result<bool, AuthorizationClientError> {
        Ok(true)
    }
}

struct HttpAuthorizationClient {
    url: String,
    client: HttpClient,
}

impl HttpAuthorizationClient {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            client: HttpClient::default(),
        }
    }
}

/// Failures which occur when checking token validity.
#[derive(Debug, Error)]
pub enum AuthorizationClientError {
    #[error(
        "Failed to check token validity against the authorization service. You should try again later,
        if the problem persists you may want to contact the operators."
    )]
    Recoverable,
}

impl AuthorizationClient for HttpAuthorizationClient {
    async fn token_valid(
        &self,
        auth: Authentication,
        context: TracingContext,
    ) -> Result<bool, AuthorizationClientError> {
        #[derive(Deserialize)]
        struct Response {
            permissions: Vec<Permission>,
        }

        #[derive(Debug, Deserialize, Eq, PartialEq, Serialize, Clone)]
        #[serde(tag = "permission")]
        enum Permission {
            KernelAccess,
        }

        let required_permissions = [Permission::KernelAccess];

        let Some(token) = auth.0 else {
            return Ok(false);
        };

        let mut builder = self
            .client
            .post(format!("{}/check_user", self.url))
            .bearer_auth(token);

        builder = builder.headers(context.w3c_headers());

        let payload = json!({
            "permissions": required_permissions
        });
        let response = builder.json(&payload).send().await.map_err(|e| {
            error!(parent: context.span(), error = %e, "Failed to send authorization request.");
            AuthorizationClientError::Recoverable
        })?;

        // Response succeeded, but not allowed
        if [StatusCode::FORBIDDEN, StatusCode::UNAUTHORIZED].contains(&response.status()) {
            return Ok(false);
        }

        let allowed_permissions = response
            // Error for any other status
            .error_for_status()
            .map_err(|e| {
                error!(parent: context.span(), error = %e, "Unexpected status code from authorization service.");
                AuthorizationClientError::Recoverable
            })?
            .json::<Response>()
            .await
            .map_err(|e| {
                error!(parent: context.span(), error = %e, "Failed to deserialize the response from the authorization service.");
                AuthorizationClientError::Recoverable
            })?;

        // Check that we got the same list back
        Ok(allowed_permissions.permissions == required_permissions)
    }
}

/// Authorization Provider allows to fetch the authorization API from an aggregated application
/// state.
pub trait AuthorizationProvider {
    type Authorization;
    fn authorization(&self) -> &Self::Authorization;
}

/// Wrapper around Authorization API for the shell. We use this strict alias to enable extracting a
/// reference from the [`AppState`] using a [`FromRef`] implementation.
pub struct AuthorizationState<M>(pub M);

impl<T> FromRef<T> for AuthorizationState<T::Authorization>
where
    T: AuthorizationProvider,
    T::Authorization: Clone,
{
    fn from_ref(app_state: &T) -> AuthorizationState<T::Authorization> {
        AuthorizationState(app_state.authorization().clone())
    }
}

#[cfg(test)]
pub mod tests {

    use std::time::Duration;

    use tokio::time::timeout;

    use super::*;
    use crate::tests::{api_token, authorization_url};

    impl Authentication {
        pub fn none() -> Self {
            Self(None)
        }
    }

    #[derive(Debug, Clone)]
    pub struct StubAuthorization {
        /// Whether the permission check should succeed or not
        response: bool,
    }

    impl StubAuthorization {
        pub fn new(response: bool) -> Self {
            Self { response }
        }
    }

    impl AuthorizationApi for StubAuthorization {
        async fn check_permission(
            &self,
            _auth: Authentication,
            _context: TracingContext,
        ) -> Result<bool, AuthorizationClientError> {
            Ok(self.response)
        }
    }

    #[tokio::test]
    async fn true_for_valid_permissions() {
        // Given a client that is configured against the inference api
        let url = authorization_url();
        let client = HttpAuthorizationClient::new(url.to_owned());
        let auth = Authentication::new(api_token());

        // When the client is used to check a valid api token
        let result = client.token_valid(auth, TracingContext::dummy()).await;

        // Then the result is true
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn false_for_invalid_token() {
        // Given a client that is configured against the inference api
        let url = authorization_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check an invalid api token
        let result = client
            .token_valid(Authentication::none(), TracingContext::dummy())
            .await;

        // Then the result is false
        assert!(!result.unwrap());
    }

    struct StubAuthorizationClient;

    impl AuthorizationClient for StubAuthorizationClient {
        async fn token_valid(
            &self,
            _auth: Authentication,
            _context: TracingContext,
        ) -> Result<bool, AuthorizationClientError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn auth_api_answers_messages() {
        // Given a stub authorization client
        let client = StubAuthorizationClient;
        let auth = Authorization::with_client(client);

        // When checking permissions for a token
        let result = timeout(
            Duration::from_millis(10),
            auth.api()
                .check_permission(Authentication::none(), TracingContext::dummy()),
        )
        .await;

        // Then the message get's answered within 10ms
        assert!(result.unwrap().unwrap());
    }

    #[tokio::test]
    async fn always_valid_client_always_returns_true() {
        // Given an authorization booted up without any url
        let auth = Authorization::new(None);

        // When checking permissions for a token
        let result = auth
            .api()
            .check_permission(Authentication::none(), TracingContext::dummy())
            .await;

        // Then the result is true
        assert!(result.unwrap());
    }

    #[tokio::test]
    async fn no_token_is_not_valid() {
        // Given a client that is configured against the inference api
        let url = authorization_url();
        let client = HttpAuthorizationClient::new(url.to_owned());

        // When the client is used to check a none token
        let result = client
            .token_valid(Authentication::none(), TracingContext::dummy())
            .await
            .unwrap();

        // Then the result is false
        assert!(!result);
    }
}
