use axum::{routing::get, Router};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("localhost:8081")
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
