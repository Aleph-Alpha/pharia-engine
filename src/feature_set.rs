use std::{convert::Infallible, str::FromStr};
use tracing::warn;

#[allow(dead_code)] // currently only referenced in shell test
pub const PRODUCTION_FEATURE_SET: FeatureSet = FeatureSet::Stable(1);

/// Pharia AI feature set as derived from the environment. It is used to hide features under
/// development from users in stable environments. It is also used to synchronize feature releases
/// across teams in Pharia AI
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureSet {
    /// We use beta environments to demo unstable features to our stakeholders.
    Beta,
    /// Our `p-prod` environment and customer installations are examples of stable environments. We
    /// only want stable features to be exposed there. We represent stable environments with an
    /// integer we increment with each feature release. This way we can synchronize feature
    /// releases.
    Stable(u32),
}

impl FromStr for FeatureSet {
    type Err = Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("BETA") {
            Ok(FeatureSet::Beta)
        } else if let Ok(n) = s.parse() {
            Ok(FeatureSet::Stable(n))
        } else {
            warn!("Failed to parse feature set: '{}. Falling back to stable feature set.", s);
            Ok(PRODUCTION_FEATURE_SET)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_feature_set() {
        let feature_set = FeatureSet::from_str("42").unwrap();
        assert_eq!(feature_set, FeatureSet::Stable(42));
    }
}
