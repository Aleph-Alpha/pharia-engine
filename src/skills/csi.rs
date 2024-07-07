use crate::inference::CompleteTextParameters;

use async_trait::async_trait;

#[async_trait]
pub trait Csi {
    async fn complete_text(&mut self, params: CompleteTextParameters) -> String;
}
