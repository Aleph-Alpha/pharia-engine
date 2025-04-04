use std::borrow::Cow;

use bytesize::ByteSize;
use dashmap::DashMap;
use moka::sync::Cache;
use wasmtime::CacheStore;

fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[divan::bench_group]
mod compilation_time {
    use std::sync::Arc;

    use bytesize::ByteSize;
    use divan::{Bencher, counter::BytesCount};
    use engine_room::Engine;
    use test_skills::{given_python_skill_greet_v0_3, given_rust_skill_greet_v0_3};
    use wasmtime::CacheStore;

    use crate::{DashMapCache, MokaCache};

    const MIN_CACHE_ENTRY: &[usize] = &[0, 8, 16, 32, 64, 128, 256, 512, 1024];
    const MAX_CACHE_SIZE: &[ByteSize] = &[
        ByteSize::mib(64),
        ByteSize::mib(128),
        ByteSize::mib(256),
        ByteSize::mib(512),
        ByteSize::gib(1),
    ];
    const NUM_SKILLS: &[usize] = &[1, 2, 3];

    #[divan::bench(sample_count = 25, consts=NUM_SKILLS)]
    fn python_base<const N: usize>(bencher: Bencher<'_, '_>) {
        bench::<N>(bencher, || given_python_skill_greet_v0_3().bytes(), || None);
    }

    #[divan::bench(consts=NUM_SKILLS)]
    fn rust_base<const N: usize>(bencher: Bencher<'_, '_>) {
        bench::<N>(bencher, || given_rust_skill_greet_v0_3().bytes(), || None);
    }

    #[divan::bench(ignore, sample_count = 25, args=MIN_CACHE_ENTRY, consts=NUM_SKILLS)]
    fn python_incremental<const N: usize>(bencher: Bencher<'_, '_>, min_cache_entry: usize) {
        bench::<N>(
            bencher,
            || given_python_skill_greet_v0_3().bytes(),
            || Some(Arc::new(DashMapCache::new(min_cache_entry))),
        );
    }

    #[divan::bench(ignore, args=MIN_CACHE_ENTRY, consts=NUM_SKILLS)]
    fn rust_incremental<const N: usize>(bencher: Bencher<'_, '_>, min_cache_entry: usize) {
        bench::<N>(
            bencher,
            || given_rust_skill_greet_v0_3().bytes(),
            || Some(Arc::new(DashMapCache::new(min_cache_entry))),
        );
    }

    #[divan::bench(sample_count = 25, args=MAX_CACHE_SIZE, consts=NUM_SKILLS)]
    fn python_lfu<const N: usize>(bencher: Bencher<'_, '_>, max_cache_size: ByteSize) {
        bench::<N>(
            bencher,
            || given_python_skill_greet_v0_3().bytes(),
            || Some(Arc::new(MokaCache::new(max_cache_size))),
        );
    }

    #[divan::bench(args=MAX_CACHE_SIZE, consts=NUM_SKILLS)]
    fn rust_lfu<const N: usize>(bencher: Bencher<'_, '_>, max_cache_size: ByteSize) {
        bench::<N>(
            bencher,
            || given_rust_skill_greet_v0_3().bytes(),
            || Some(Arc::new(MokaCache::new(max_cache_size))),
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
struct MokaCache {
    cache: Cache<Vec<u8>, Vec<u8>>,
}

impl MokaCache {
    fn new(max_cache_size: ByteSize) -> Self {
        Self {
            cache: Cache::builder()
                .weigher(|_, v: &Vec<u8>| v.len().try_into().unwrap_or(u32::MAX))
                .max_capacity(max_cache_size.as_u64())
                .build(),
        }
    }
}

impl CacheStore for MokaCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.cache.get(key).map(Into::into)
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        self.cache.insert(key.to_vec(), value);
        true
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
