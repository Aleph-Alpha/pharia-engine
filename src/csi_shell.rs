/// Expose the CSI (Cognitive System Interface which is offered to skills) via HTTP.
///
/// This allows users to test and debug Skills on their machine while still having access
/// to the full functionality the Kernel offers as a runtime environment.
mod v0_2;
mod v0_3;

use axum::{extract::State, http::StatusCode, Json};
use axum_extra::{
    headers::{authorization::Bearer, Authorization},
    TypedHeader,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::csi_shell::v0_2::CsiRequest as V0_2CsiRequest;
use crate::csi_shell::v0_3::CsiRequest as V0_3CsiRequest;
use crate::{csi::Csi, shell::AppState};

#[allow(clippy::too_many_lines)]
pub async fn http_csi_handle<C>(
    State(app_state): State<AppState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>)
where
    C: Csi + Clone + Sync,
{
    let drivers = app_state.csi_drivers;
    let result = match args {
        VersionedCsiRequest::V0_2(request) => {
            request.act(&drivers, bearer.token().to_owned()).await
        }
        VersionedCsiRequest::V0_3(request) => {
            request.act(&drivers, bearer.token().to_owned()).await
        }
    };
    match result {
        Ok(result) => (StatusCode::OK, Json(result)),
        Err(e) => (e.status_code(), Json(json!(e.to_string()))),
    }
}

/// This represents the versioned interactions with the CSI.
/// The members of this enum provide the glue code to translate between a function
/// defined in a versioned WIT world and the `CsiForSkills` trait.
/// By introducing this abstraction, we can expose a versioned interface of the CSI over http.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", tag = "version")]
pub enum VersionedCsiRequest {
    #[serde(rename = "0.3")]
    V0_3(V0_3CsiRequest),
    #[serde(rename = "0.2")]
    V0_2(V0_2CsiRequest),
}

#[derive(Debug, thiserror::Error)]
pub enum CsiShellError {
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl CsiShellError {
    fn status_code(&self) -> StatusCode {
        match self {
            CsiShellError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl From<CsiShellError> for (StatusCode, Json<Value>) {
    fn from(e: CsiShellError) -> Self {
        (e.status_code(), Json(json!(e.to_string())))
    }
}
