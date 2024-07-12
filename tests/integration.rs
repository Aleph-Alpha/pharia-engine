use std::{env, sync::OnceLock};

use dotenvy::dotenv;
use pharia_kernel::{run, AppConfig};
use tokio::sync::oneshot;

#[cfg_attr(not(feature = "test_inference"), ignore)]
#[tokio::test]
async fn execute_skill() {
    let api_token = api_token();

    let app_config = AppConfig {
        tcp_addr: "127.0.0.1:9000".to_string().parse().unwrap(),
        inference_addr: "https://api.aleph-alpha.com".to_owned(),
    };

    let (send, recv) = oneshot::channel::<()>();
    let run_handle = tokio::spawn(run(app_config, async move { recv.await.unwrap() }));

    // let mut auth_value = header::HeaderValue::from_str(&format!("Bearer {api_token}")).unwrap();
    // auth_value.set_sensitive(true);
    //
    // let inference = Inference::new(inference_addr().to_owned());
    // let http = http(SkillExecutor::new(inference.api()).api());
    //
    // let args = ExecuteSkillArgs {
    //     skill: "greet_skill".to_owned(),
    //     input: "Homer".to_owned(),
    // };
    // let resp = http
    //     .oneshot(
    //         Request::builder()
    //             .method(http::Method::POST)
    //             .header(http::header::CONTENT_TYPE, mime::APPLICATION_JSON.as_ref())
    //             .header(header::AUTHORIZATION, auth_value)
    //             .uri("/execute_skill")
    //             .body(Body::from(serde_json::to_string(&args).unwrap()))
    //             .unwrap(),
    //     )
    //     .await
    //     .unwrap();
    // assert_eq!(resp.status(), axum::http::StatusCode::OK);
    // let body = resp.into_body().collect().await.unwrap().to_bytes();
    // let answer = String::from_utf8(body.to_vec()).unwrap();
    // assert!(answer.contains("Homer"));
}

/// API Token used by tests to authenticate requests
fn api_token() -> &'static str {
    static API_TOKEN: OnceLock<String> = OnceLock::new();
    API_TOKEN.get_or_init(|| {
        drop(dotenv());
        env::var("AA_API_TOKEN").expect("AA_API_TOKEN variable not set")
    })
}
