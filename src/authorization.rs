#![expect(dead_code)]
use axum::{
    body::Body,
    http::Request,
    middleware::Next,
    response::{ErrorResponse, Response},
};
use axum_extra::{
    headers::{self, authorization::Bearer},
    TypedHeader,
};
use tokio::{sync::mpsc, task::JoinHandle};

pub struct Authorization {
    send: mpsc::Sender<AuthorizationMsg>,
    handle: JoinHandle<()>,
}

impl Authorization {
    pub fn new() -> Authorization {
        let (send, _recv) = mpsc::channel(3);
        let handle = tokio::spawn(async move {});
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

#[derive(Clone)]
pub struct AuthorizationApi {
    sender: mpsc::Sender<AuthorizationMsg>,
}

impl AuthorizationApi {
    pub fn new(sender: mpsc::Sender<AuthorizationMsg>) -> Self {
        Self { sender }
    }
}

pub enum AuthorizationMsg {}

pub async fn authorization_middleware(
    _bearer: Option<TypedHeader<headers::Authorization<Bearer>>>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ErrorResponse> {
    let response = next.run(request).await;
    Ok(response)
}

#[cfg(test)]
pub mod tests {

    use super::*;

    pub fn dummy_authorization_api() -> AuthorizationApi {
        let (send, _recv) = mpsc::channel(1);
        AuthorizationApi::new(send)
    }
}
