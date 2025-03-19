/// Expose the CSI (Cognitive System Interface which is offered to skills) via HTTP.
///
/// This allows users to test and debug Skills on their machine while still having access
/// to the full functionality the Kernel offers as a runtime environment.
mod v0_2;
mod v0_3;

use axum::{Json, extract::State, http::StatusCode};
use axum_extra::{
    TypedHeader,
    headers::{Authorization, authorization::Bearer},
};
use semver::VersionReq;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    csi::Csi,
    csi_shell::{v0_2::CsiRequest as V0_2CsiRequest, v0_3::CsiRequest as V0_3CsiRequest},
    shell::CsiState,
    skills::SupportedVersion,
};

pub async fn http_csi_handle<C>(
    State(CsiState(csi)): State<CsiState<C>>,
    bearer: TypedHeader<Authorization<Bearer>>,
    Json(args): Json<VersionedCsiRequest>,
) -> (StatusCode, Json<Value>)
where
    C: Csi + Clone + Sync,
{
    let drivers = csi;
    let result = match args {
        VersionedCsiRequest::V0_2(request) => {
            request.respond(&drivers, bearer.token().to_owned()).await
        }
        VersionedCsiRequest::V0_3(request) => {
            request.respond(&drivers, bearer.token().to_owned()).await
        }
        VersionedCsiRequest::Unknown(request) => Err(request.into()),
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
    #[serde(untagged)]
    Unknown(UnknownCsiRequest),
}

#[derive(Debug, thiserror::Error)]
pub enum CsiShellError {
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
    #[error(
        "The CSI function {0} is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK."
    )]
    UnknownFunction(String),
    #[error(
        "The specified CSI version is not supported by this Kernel installation yet. Try updating your Kernel version or downgrading your SDK."
    )]
    NotSupported,
    #[error("This CSI version is no longer supported by the Kernel. Try upgrading your SDK.")]
    NoLongerSupported,
    #[error("A valid CSI version is required. Try upgrading your SDK.")]
    InvalidVersion,
}

impl CsiShellError {
    fn status_code(&self) -> StatusCode {
        match self {
            CsiShellError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
            // We use `BAD_REQUEST` (400) for validation error as it is more commonly used.
            // `UNPROCESSABLE_ENTITY` (422) is an alternative, but it may surprise users as it is less commonly
            // known
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl From<CsiShellError> for (StatusCode, Json<Value>) {
    fn from(e: CsiShellError) -> Self {
        (e.status_code(), Json(json!(e.to_string())))
    }
}

#[derive(Deserialize)]
pub struct UnknownCsiRequest {
    version: Option<String>,
}

impl From<UnknownCsiRequest> for CsiShellError {
    fn from(e: UnknownCsiRequest) -> Self {
        match e.version.map(|v| VersionReq::parse(&v)) {
            Some(Ok(req)) if req.comparators.len() == 1 => {
                let max_supported_version = SupportedVersion::latest_supported_version();
                let comp = req.comparators.first().unwrap();
                // Only applies to unknown versions. If we parse `1.x.x` as `1` then we are only doing a major version check and minor version only applies `0.x`
                if comp.major > max_supported_version.major
                    || (comp.major == max_supported_version.major
                        && comp.minor.is_some_and(|m| m > max_supported_version.minor))
                {
                    CsiShellError::NotSupported
                } else {
                    CsiShellError::NoLongerSupported
                }
            }
            // If the user passes in a random string, the parse will fail and we will end up down here
            Some(Ok(_) | Err(_)) | None => CsiShellError::InvalidVersion,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_longer_supported_version() {
        let request = UnknownCsiRequest {
            version: Some("0.1.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NoLongerSupported));
    }

    #[test]
    fn not_supported_minor_version() {
        let request = UnknownCsiRequest {
            version: Some("0.9.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NotSupported));
    }

    #[test]
    fn not_supported_major_version() {
        let request = UnknownCsiRequest {
            version: Some("1.0.0".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::NotSupported));
    }

    #[test]
    fn invalid_version() {
        let request = UnknownCsiRequest {
            version: Some("invalid".to_string()),
        };
        let error: CsiShellError = request.into();
        assert!(matches!(error, CsiShellError::InvalidVersion));
    }
}
