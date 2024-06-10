use axum::{routing::get, Router};
use tokio::net::TcpListener;
use std::env;


#[tokio::main]
async fn main() {
    // .env files are optional
    let _ = dotenvy::dotenv();

    let host = env::var("HOST").expect("HOST variable not set");
    let port = env::var("PORT").expect("PORT variable not set");
    let bind = format!("{host}:{port}");
    let listener = TcpListener::bind(bind)
        .await
        .expect("Could not bind server, please check host and port"); //todo:
                                                                      //error handling

    axum::serve(listener, http())
        .await
        .expect("Could not start server!"); //todo: error handling
}

fn http() -> Router {
    Router::new().route("/", get(|| async { "Hello, world!" }))
}

#[cfg(test)]
mod tests {

    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn hello_world() {
        let http = http();
        let resp = http
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&body[..], b"Hello, world!");
    }
}
