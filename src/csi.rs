use crate::inference::InferenceApi;

/// Collection of api handles to the actors used to implement the Cognitive System Interface (CSI)
/// 
/// For now this is just a collection of all the APIs without providing logic on its own
pub struct CsiApis {
    /// We use the inference Api to complete text
    pub inference: InferenceApi
}