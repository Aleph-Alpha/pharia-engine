use std::borrow::Cow;

use dashmap::DashMap;
use wasmtime::CacheStore;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[divan::bench_group]
mod compilation_time {
    use std::sync::Arc;

    use divan::{Bencher, counter::BytesCount};
    use engine_room::Engine;
    use test_skills::{given_python_skill_greet_v0_3, given_rust_skill_greet_v0_3};
    use wasmtime::CacheStore;

    use crate::DashMapCache;

    const MIN_CACHE_ENTRY: &[usize] = &[0, 8, 64, 512, 4096, 32768];
    const NUM_SKILLS: &[usize] = &[1, 2, 3];

    #[divan::bench(sample_count = 10, consts=NUM_SKILLS)]
    fn python_base<const N: usize>(bencher: Bencher<'_, '_>) {
        bench::<N>(bencher, || given_python_skill_greet_v0_3().bytes(), || None);
    }

    #[divan::bench(consts=NUM_SKILLS)]
    fn rust_base<const N: usize>(bencher: Bencher<'_, '_>) {
        bench::<N>(bencher, || given_rust_skill_greet_v0_3().bytes(), || None);
    }

    #[divan::bench(sample_count = 10, args=MIN_CACHE_ENTRY, consts=NUM_SKILLS)]
    fn python_base_incremental<const N: usize>(bencher: Bencher<'_, '_>, min_cache_entry: usize) {
        bench::<N>(
            bencher,
            || given_python_skill_greet_v0_3().bytes(),
            || Some(Arc::new(DashMapCache::new(min_cache_entry))),
        );
    }

    #[divan::bench(args=MIN_CACHE_ENTRY, consts=NUM_SKILLS)]
    fn rust_base_incremental<const N: usize>(bencher: Bencher<'_, '_>, min_cache_entry: usize) {
        bench::<N>(
            bencher,
            || given_rust_skill_greet_v0_3().bytes(),
            || Some(Arc::new(DashMapCache::new(min_cache_entry))),
        );
    }

    /// Shared benchmark setup
    /// Benchmarks creation of a component. Doesn't include instantiation/linking
    fn bench<const N: usize>(
        bencher: Bencher<'_, '_>,
        skill: impl Fn() -> Vec<u8> + Sync,
        cache_store: impl Fn() -> Option<Arc<dyn CacheStore>> + Sync,
    ) {
        bencher
            .with_inputs(|| (Engine::new(false, cache_store()).unwrap(), skill()))
            .input_counter(|(_, bytes)| BytesCount::new(bytes.len() * N))
            .bench_values(|(engine, bytes)| {
                for _ in 0..N {
                    engine.new_component(&bytes).unwrap();
                }
            });
    }
}

#[derive(Debug)]
struct DashMapCache {
    min_cache_entry: usize,
    cache: DashMap<Vec<u8>, Vec<u8>>,
}

impl DashMapCache {
    fn new(min_cache_entry: usize) -> Self {
        Self {
            cache: DashMap::new(),
            min_cache_entry,
        }
    }
}

impl CacheStore for DashMapCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.cache.get(key).map(|v| v.clone().into())
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        if value.len() >= self.min_cache_entry {
            self.cache.insert(key.to_vec(), value);
        }
        true
    }
}
