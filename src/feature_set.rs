use serde::{Deserialize, Deserializer, de::Visitor};
use tracing::warn;

#[allow(dead_code)] // currently only referenced in shell test
pub const PRODUCTION_FEATURE_SET: FeatureSet = FeatureSet::Stable(1);

/// Pharia AI feature set as derived from the environment. It is used to hide features under
/// development from users in stable environments. It is also used to synchronize feature releases
/// across teams in Pharia AI
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FeatureSet {
    /// We use beta environments to demo unstable features to our stakeholders.
    Beta,
    /// Our `p-prod` environment and customer installations are examples of stable environments. We
    /// only want stable features to be exposed there. We represent stable environments with an
    /// integer we increment with each feature release. This way we can synchronize feature
    /// releases.
    Stable(u32),
}

struct FeatureSetVisitor;

impl Visitor<'_> for FeatureSetVisitor {
    type Value = FeatureSet;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("the string \"BETA\" or an integer indicates the stable version")
    }

    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let version = u32::try_from(v).map_err(E::custom)?;
        Ok(FeatureSet::Stable(version))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if v.eq_ignore_ascii_case("BETA") {
            Ok(FeatureSet::Beta)
        } else if let Ok(v) = v.parse() {
            self.visit_u32(v)
        } else {
            warn!(
                "Failed to parse feature set: '{}'. Falling back to BETA.",
                v
            );
            Ok(FeatureSet::Beta)
        }
    }
}

impl<'de> Deserialize<'de> for FeatureSet {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(FeatureSetVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_feature_set() {
        let feature_set = serde_json::from_str::<FeatureSet>("42").unwrap();
        assert_eq!(feature_set, FeatureSet::Stable(42));
    }
}
