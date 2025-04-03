use std::sync::Arc;

use metrics::Gauge;
use tokio::time::Instant;

use crate::{
    registries::Digest,
    skill_loader::LoadedSkill,
    skills::{Skill, SkillPath},
};

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
struct CachedSkill {
    /// Compiled and pre-initialized skill
    skill: Arc<dyn Skill>,
    /// Digest of the skill when it was last loaded from the registry
    digest: Digest,
    /// When we last checked the digest
    digest_validated: Instant,
    /// The weight of the item in the cache, to know if we need to evict something.
    weight: u64,
}

impl CachedSkill {
    fn new(skill: Arc<dyn Skill>, digest: Digest, weight: u64) -> Self {
        Self {
            skill,
            digest,
            digest_validated: Instant::now(),
            weight,
        }
    }
}

struct SkillWeighter;

impl quick_cache::Weighter<SkillPath, CachedSkill> for SkillWeighter {
    fn weight(&self, _key: &SkillPath, val: &CachedSkill) -> u64 {
        val.weight
    }
}

pub enum SkillCacheMetrics {
    Items,
}

impl From<SkillCacheMetrics> for metrics::KeyName {
    fn from(value: SkillCacheMetrics) -> Self {
        Self::from_const_str(match value {
            SkillCacheMetrics::Items => "kernel_skill_cache_items",
        })
    }
}

/// Uses a `quick_cache::unsync::Cache` to store skills.
/// We can use unsync because we are already managing this in a single-threaded environment.
/// This means we can use a more performant version without overhead for multiple threads.
pub struct SkillCache {
    cache: quick_cache::unsync::Cache<SkillPath, CachedSkill, SkillWeighter>,
    gauge: Gauge,
}

impl SkillCache {
    /// One of our Python Wasm Skills is roughly 60MB in size, on average.
    const PYTHON_SKILL_SIZE: u64 = 60 * 1024 * 1024;

    /// Create a new `SkillCache` that can hold approximately up to `capacity` skills.
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: quick_cache::unsync::Cache::with_weighter(
                capacity,
                Self::PYTHON_SKILL_SIZE.saturating_mul(capacity as u64),
                SkillWeighter,
            ),
            gauge: metrics::gauge!(SkillCacheMetrics::Items),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &SkillPath> + '_ {
        self.cache.iter().map(|(key, _)| key)
    }

    fn update_gauge(&self) {
        #[expect(clippy::cast_precision_loss)]
        self.gauge.set(self.cache.len() as f64);
    }

    pub fn get(&self, skill_path: &SkillPath) -> Option<Arc<dyn Skill>> {
        self.cache.get(skill_path).map(|skill| skill.skill.clone())
    }

    pub fn insert(&mut self, skill_path: SkillPath, compiled_skill: LoadedSkill) {
        let LoadedSkill {
            skill,
            digest,
            size_loaded_from_registry,
        } = compiled_skill;
        self.cache.insert(
            skill_path,
            CachedSkill::new(skill, digest, size_loaded_from_registry),
        );
        self.update_gauge();
    }

    pub fn remove(&mut self, skill_path: &SkillPath) -> bool {
        let removed = self.cache.remove(skill_path).is_some();
        self.update_gauge();
        removed
    }

    /// Retrieve the oldest digest validation timestamp. So, the one we would need to refresh the soonest.
    /// If there are no cached skills, it will return `None`.
    pub fn oldest_digest(&self) -> Option<(SkillPath, Instant)> {
        self.cache
            .iter()
            .min_by_key(|(_, c)| c.digest_validated)
            .map(|(skill_path, cached_skill)| (skill_path.clone(), cached_skill.digest_validated))
    }

    /// Just mark the digest as validated.
    /// Useful in cases where we were unable to retrieve the latest digest from the registry, and we want to update
    /// the timestamp so that we don't try to refresh it again too soon.
    pub fn update_digest_validated(&mut self, skill_path: &SkillPath) {
        if let Some(mut cached_skill) = self.cache.get_mut(skill_path) {
            cached_skill.digest_validated = Instant::now();
        }
    }

    /// Compares the digest in the cache with the digest behind the corresponding tag in the registry.
    /// If the digest behind the tag has changed, remove the cache entry.
    pub fn compare_latest_digest(
        &mut self,
        skill_path: &SkillPath,
        latest_digest: &Digest,
    ) -> anyhow::Result<()> {
        let CachedSkill { digest, .. } = self
            .cache
            .get(skill_path)
            .ok_or_else(|| anyhow::anyhow!("Missing cached skill for {skill_path}"))?;

        // There is a new digest behind the tag, delete the cache entry
        if latest_digest != digest {
            self.cache.remove(skill_path);
        }
        self.update_digest_validated(skill_path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::skills::tests::SkillDummy;

    use super::*;

    #[test]
    fn cache_invalidation() {
        let capacity = 2;
        let mut cache = SkillCache::new(capacity);
        let loaded_skill = LoadedSkill::new(
            Arc::new(SkillDummy),
            Digest::new("digest"),
            SkillCache::PYTHON_SKILL_SIZE,
        );

        let skill_paths = [SkillPath::dummy(), SkillPath::dummy(), SkillPath::dummy()];

        // Insert the skill three times at different paths
        cache.insert(skill_paths[0].clone(), loaded_skill.clone());
        cache.insert(skill_paths[1].clone(), loaded_skill.clone());
        cache.insert(skill_paths[2].clone(), loaded_skill);

        let keys = cache.keys().collect::<Vec<_>>();
        // One was evicted
        assert_eq!(keys.len(), capacity);
        // Most recent inserted skill should be in the cache
        assert!(keys.contains(&&skill_paths[2]));
    }

    #[test]
    fn rust_skills_evicted_less() {
        let capacity = 10;
        let rust_size = SkillCache::PYTHON_SKILL_SIZE / 100;
        let mut cache = SkillCache::new(capacity);
        let loaded_skill = LoadedSkill::new(Arc::new(SkillDummy), Digest::new("digest"), rust_size);

        let skill_paths = (0..100).map(|_| SkillPath::dummy()).collect::<Vec<_>>();

        for skill_path in &skill_paths {
            cache.insert(skill_path.clone(), loaded_skill.clone());
        }

        let keys = cache.keys().collect::<Vec<_>>();
        // Can hold more than capacity
        assert!(keys.len() > capacity * 5);
    }
}
