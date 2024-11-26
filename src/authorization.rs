#![expect(dead_code)]
use std::{future::Future, pin::Pin};

use axum::{
    body::Body,
    extract::State,
    http::Request,
    http::StatusCode,
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
    pub fn new() -> Authorization {
        let (send, recv) = mpsc::channel(1);
        let mut actor = AuthorizationActor::new(recv);
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

struct AuthorizationActor {
    recv: mpsc::Receiver<AuthorizationMsg>,
    running_authorizations: FuturesUnordered<Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl AuthorizationActor {
    fn new(recv: mpsc::Receiver<AuthorizationMsg>) -> Self {
        Self {
            recv,
            running_authorizations: FuturesUnordered::new(),
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
        self.running_authorizations
            .push(Box::pin(async move { msg.act() }));
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
    fn act(self) {
        match self {
            AuthorizationMsg::Auth { api_token: _, send } => drop(send.send(Ok(true))),
        }
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

#[cfg(test)]
pub mod tests {

    use super::*;

    /// An authorization double, loaded up with predefined answers.
    pub struct StubAuthorization {
        send: mpsc::Sender<AuthorizationMsg>,
        handle: JoinHandle<()>,
    }

    impl StubAuthorization {
        pub fn new(mut handle: impl FnMut(AuthorizationMsg) + Send + 'static) -> Self {
            let (send, mut recv) = mpsc::channel(1);
            let handle = tokio::spawn(async move {
                while let Some(msg) = recv.recv().await {
                    handle(msg);
                }
            });
            Self { send, handle }
        }

        pub fn api(&self) -> AuthorizationApi {
            AuthorizationApi::new(self.send.clone())
        }

        pub async fn shutdown(self) {
            drop(self.send);
            self.handle.await.unwrap();
        }
    }
}
