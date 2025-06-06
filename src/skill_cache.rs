use std::sync::Arc;

use bytesize::ByteSize;
use metrics::Gauge;
use moka::{ops::compute::Op, sync::Cache};
use tokio::time::Instant;
use tracing::info;

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
    /// Create a new `SkillCache` that can hold approximately up to `desired_memory_usage`.
    /// It is really hard to predict the exact memory usage of the cache, so we use a heuristic
    /// to estimate the capacity based on the desired memory usage.
    /// We define weight of a skill as the size of the wasm module loaded from the registry in bytes.
    pub fn new(desired_memory_usage: ByteSize) -> Self {
        let capacity = Self::estimated_capacity(desired_memory_usage);
        Self {
            cache: Cache::builder()
                .weigher(|_, cached_skill: &CachedSkill| cached_skill.weight)
                .max_capacity(capacity.as_u64())
                .eviction_listener(|key, _, removal_cause| {
                    if removal_cause.was_evicted() {
                        info!("Cache: {key} evicted. Cause: {:?}", removal_cause);
                    }
                })
                .build(),
            gauge: metrics::gauge!(SkillCacheMetrics::Items),
        }
    }

    /// Generates a capacity that should end up around the desired target memory usage.
    /// We use bytes of wasm modules as a proxy for the memory usage of the cache, since we can't see exactly
    /// how much memory a skill is using.
    ///
    /// It isn't perfect, because compilation artifacts can lead to some heap fragmentation, so we are roughly
    /// measuring not only how much the skill takes in memory, but what the memory pressure is after compiling it.
    /// (This is why we switched to jemalloc, to help with this).
    ///
    /// This is a best effort estimate and may need to be adjusted based on the actual measurements in production.
    ///
    /// The following measurements are based on loading a number of Python skills from our SDK on my Mac (Ben).
    /// But it also seems to correlate with what we see in production on Linux. So we'll use it as a guide.
    /// Incremental compilation and jemalloc were used.
    ///
    /// 01. 0.73GB
    /// 02. 0.85GB ~ 0.12GB
    /// 03. 0.97GB ~ 0.12GB
    /// 04. 1.03GB ~ 0.06GB
    /// 05. 1.08GB ~ 0.05GB
    /// 06. 1.16GB ~ 0.08GB
    /// 07. 1.23GB ~ 0.07GB
    /// 08. 1.29GB ~ 0.06GB
    /// 09. 1.35GB ~ 0.06GB
    /// 10. 1.42GB - 0.07GB
    /// 11. 1.50GB - 0.08GB
    /// 12. 1.60GB - 0.10GB
    /// 13. 1.69GB - 0.09GB
    ///
    /// Assuming the large initial allocation is from compilation artifacts (validated the theory with Joel Dice and
    /// Alex Chrichton), it is possible this memory will get reused later. So the actual memory pressure from the
    /// cache is closer to skill 2+ and beyond.
    ///
    /// The skill used for my test was 38.5MB on disk, so this would be roughly a factor of ~2x memory vs bytes of
    /// the component.
    ///
    /// Given this, for 2GB of desired memory, we can store roughly 17 skills. Which, extrapolating from the data out
    /// to 17 skills, this lines up.
    fn estimated_capacity(desired_memory_usage: ByteSize) -> ByteSize {
        ByteSize(desired_memory_usage.as_u64() / 2)
    }

    pub fn keys(&self) -> impl Iterator<Item = SkillPath> + '_ {
        self.cache.iter().map(|(key, _)| key.as_ref().clone())
    }

    fn update_gauge(&self) {
        #[expect(clippy::cast_precision_loss)]
        self.gauge.set(self.cache.entry_count() as f64);
    }

    pub fn get(&self, skill_path: &SkillPath) -> Option<Arc<dyn Skill>> {
        // The entry count is estimated, so update on gets as well to avoid stale values.
        self.update_gauge();
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
            CachedSkill::new(
                skill,
                digest,
                u32::try_from(size_loaded_from_registry.as_u64()).unwrap_or(u32::MAX),
            ),
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
        self.update_gauge();
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
        self.update_gauge();
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

    use double_trait::Dummy;

    use super::*;

    #[test]
    fn cache_invalidation() {
        let desired_memory_usage = ByteSize::mib(240);
        let mut cache = SkillCache::new(desired_memory_usage);
        let loaded_skill =
            LoadedSkill::new(Arc::new(Dummy), Digest::new("digest"), ByteSize::mib(60));

        let skill_paths = [SkillPath::dummy(), SkillPath::dummy(), SkillPath::dummy()];

        // Insert the skill three times at different paths
        cache.insert(skill_paths[0].clone(), loaded_skill.clone());
        cache.insert(skill_paths[1].clone(), loaded_skill.clone());
        cache.insert(skill_paths[2].clone(), loaded_skill);

        cache.cache.run_pending_tasks();
        let keys = cache.keys().collect::<Vec<_>>();
        // One was evicted
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn rust_skills_evicted_less() {
        let desired_memory_usage = ByteSize::mib(1200);
        let rust_size = ByteSize::kib(500);
        let mut cache = SkillCache::new(desired_memory_usage);
        let loaded_skill = LoadedSkill::new(Arc::new(Dummy), Digest::new("digest"), rust_size);

        let skill_paths = (0..100).map(|_| SkillPath::dummy()).collect::<Vec<_>>();

        for skill_path in &skill_paths {
            cache.insert(skill_path.clone(), loaded_skill.clone());
        }
        cache.cache.run_pending_tasks();

        let keys = cache.keys().collect::<Vec<_>>();
        assert!(keys.len() > 50);
    }

    #[test]
    fn estimated_capacity() {
        let desired_memory_usage = ByteSize::mib(1200);

        let capacity = SkillCache::estimated_capacity(desired_memory_usage);

        // 10 Skills at 60 mb each
        let diff = capacity - ByteSize::mib(600);
        // We're within 100kb of the desired memory usage.
        assert!(diff <= ByteSize::kib(100));
    }
}
