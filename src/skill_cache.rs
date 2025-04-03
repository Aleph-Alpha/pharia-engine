use std::sync::Arc;

use metrics::Gauge;
use moka::{ops::compute::Op, sync::Cache};
use tokio::time::Instant;

use crate::{
    registries::Digest,
    skill_loader::LoadedSkill,
    skills::{Skill, SkillPath},
};

/// A wrapper around the Skill to also keep track of which
/// digest was loaded last and when it was last checked.
#[derive(Clone)]
struct CachedSkill {
    /// Compiled and pre-initialized skill
    skill: Arc<dyn Skill>,
    /// Digest of the skill when it was last loaded from the registry
    digest: Arc<Digest>,
    /// When we last checked the digest
    digest_validated: Instant,
    /// The weight of the item in the cache, to know if we need to evict something.
    weight: u32,
}

impl CachedSkill {
    fn new(skill: Arc<dyn Skill>, digest: Digest, weight: u32) -> Self {
        Self {
            skill,
            digest: digest.into(),
            digest_validated: Instant::now(),
            weight,
        }
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

/// Wrapper around the `moka` crate cache. This provides smart eviction policies based on usage,
/// so we can leverage the latest papers that have been published on the topic, where it seems a
/// naive approach of using a simple LRU policy may not be the best choice.
pub struct SkillCache {
    cache: Cache<SkillPath, CachedSkill>,
    gauge: Gauge,
}

impl SkillCache {
    /// Create a new `SkillCache` that can hold approximately up to `capacity` weight.
    /// We define weight of a skill as the size of the wasm module loaded from the registry in bytes.
    pub fn new(capacity: u64) -> Self {
        Self {
            cache: Cache::builder()
                .weigher(|_, cached_skill: &CachedSkill| cached_skill.weight)
                .max_capacity(capacity)
                .build(),
            gauge: metrics::gauge!(SkillCacheMetrics::Items),
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = SkillPath> + '_ {
        self.cache.iter().map(|(key, _)| key.as_ref().clone())
    }

    fn update_gauge(&self) {
        #[expect(clippy::cast_precision_loss)]
        self.gauge.set(self.cache.entry_count() as f64);
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
    pub fn oldest_digest(&self) -> Option<(Arc<SkillPath>, Instant)> {
        self.cache
            .iter()
            .min_by_key(|(_, c)| c.digest_validated)
            .map(|(skill_path, cached_skill)| (skill_path, cached_skill.digest_validated))
    }

    /// Just mark the digest as validated.
    /// Useful in cases where we were unable to retrieve the latest digest from the registry, and we want to update
    /// the timestamp so that we don't try to refresh it again too soon.
    pub fn update_digest_validated(&mut self, skill_path: SkillPath) {
        self.cache.entry(skill_path).and_compute_with(|entry| {
            if let Some(entry) = entry {
                let mut cached_skill = entry.into_value();
                cached_skill.digest_validated = Instant::now();
                Op::Put(cached_skill)
            } else {
                Op::Nop
            }
        });
    }

    /// Compares the digest in the cache with the digest behind the corresponding tag in the registry.
    /// If the digest behind the tag has changed, remove the cache entry.
    pub fn compare_latest_digest(
        &mut self,
        skill_path: SkillPath,
        latest_digest: &Digest,
    ) -> anyhow::Result<()> {
        let CachedSkill { digest, .. } = self
            .cache
            .get(&skill_path)
            .ok_or_else(|| anyhow::anyhow!("Missing cached skill for {skill_path}"))?;

        // There is a new digest behind the tag, delete the cache entry
        if latest_digest != digest.as_ref() {
            self.cache.invalidate(&skill_path);
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
        let loaded_skill = LoadedSkill::new(Arc::new(SkillDummy), Digest::new("digest"), 1);

        let skill_paths = [SkillPath::dummy(), SkillPath::dummy(), SkillPath::dummy()];

        // Insert the skill three times at different paths
        cache.insert(skill_paths[0].clone(), loaded_skill.clone());
        cache.insert(skill_paths[1].clone(), loaded_skill.clone());
        cache.insert(skill_paths[2].clone(), loaded_skill);

        cache.cache.run_pending_tasks();
        let keys = cache.keys().collect::<Vec<_>>();
        // One was evicted
        assert_eq!(keys.len() as u64, capacity);
    }

    #[test]
    fn rust_skills_evicted_less() {
        let capacity = 100;
        let rust_size = 1;
        let mut cache = SkillCache::new(capacity);
        let loaded_skill = LoadedSkill::new(Arc::new(SkillDummy), Digest::new("digest"), rust_size);

        let skill_paths = (0..100).map(|_| SkillPath::dummy()).collect::<Vec<_>>();

        for skill_path in &skill_paths {
            cache.insert(skill_path.clone(), loaded_skill.clone());
        }
        cache.cache.run_pending_tasks();

        let keys = cache.keys().collect::<Vec<_>>();
        assert!(keys.len() > (capacity / 2) as usize);
    }
}
