//! The Pharia Kernel Engine Room
//!
//! This library hopefully contains and abstracts all of the core wasmtime knowledge needed to run
//! skills within the Pharia Kernel. There will still be bindings and linking that happens outside
//! of this library, but everything related to efficient execution of wasm components, regardless
//! of the world it targets, should live here.

use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::{Duration, Instant},
};

use bytesize::ByteSize;
use moka::sync::Cache as MokaCache;
use tracing::info;
use wasmtime::{
    Cache, CacheConfig, CacheStore, Config, Engine as WasmtimeEngine, InstanceAllocationStrategy,
    Memory, MemoryType, OptLevel, Store, UpdateDeadline,
    component::{Component, Linker},
};
use wasmtime_wasi::{
    ResourceTable,
    p2::{IoView, WasiCtx, WasiView, add_to_linker_async},
};

#[derive(Default)]
pub struct EngineConfig {
    /// How much memory you are willing to allocate for an incremental cache, in any.
    max_incremental_cache_size: Option<ByteSize>,
    /// Whether or not to use a pooling allocator for invocation memory.
    use_pooling_allocator: bool,
    /// Optional wasmtime cache settings for storing compilation artifacts on disk
    wasmtime_cache: Option<WasmtimeCache>,
}

impl EngineConfig {
    #[must_use]
    pub fn with_max_incremental_cache_size(
        mut self,
        max_incremental_cache_size: Option<ByteSize>,
    ) -> Self {
        self.max_incremental_cache_size = max_incremental_cache_size;
        self
    }

    #[must_use]
    pub fn use_pooling_allocator(mut self, use_pooling_allocator: bool) -> Self {
        self.use_pooling_allocator = use_pooling_allocator;
        self
    }

    #[must_use]
    pub fn with_wasmtime_cache(mut self, wasmtime_cache: Option<WasmtimeCache>) -> Self {
        self.wasmtime_cache = wasmtime_cache;
        self
    }
}

impl TryFrom<EngineConfig> for Config {
    type Error = anyhow::Error;

    fn try_from(value: EngineConfig) -> Result<Self, Self::Error> {
        let EngineConfig {
            max_incremental_cache_size,
            use_pooling_allocator,
            wasmtime_cache,
        } = value;

        let mut config = Self::new();
        config
            .async_support(true)
            .cranelift_opt_level(OptLevel::SpeedAndSize)
            // Allows for cooperative timeslicing in async mode
            .epoch_interruption(true)
            .wasm_component_model(true);

        if use_pooling_allocator && pooling_allocator_is_supported() {
            // For more information on Pooling Allocation, as well as all of possible configuration,
            // read the wasmtime docs: https://docs.rs/wasmtime/latest/wasmtime/struct.PoolingAllocationConfig.html
            config.allocation_strategy(InstanceAllocationStrategy::pooling());
        }

        if let Some(size) = max_incremental_cache_size {
            config
                .enable_incremental_compilation(Arc::new(IncrementalCompilationCache::new(size)))?;
        }

        if let Some(wasmtime_cache) = wasmtime_cache {
            let cache = Cache::new(wasmtime_cache.into_config())?;
            config.cache(Some(cache));
        }

        Ok(config)
    }
}

/// Settings for wasmtime file-based cache
pub struct WasmtimeCache {
    /// Where the cache should live
    directory: PathBuf,
    /// How large the cache is allowed to be
    size_limit: ByteSize,
}

impl WasmtimeCache {
    pub fn new(directory: impl Into<PathBuf>, size_limit: ByteSize) -> Self {
        Self {
            directory: directory.into(),
            size_limit,
        }
    }

    fn into_config(self) -> CacheConfig {
        let mut cache_config = CacheConfig::new();
        cache_config
            .with_directory(self.directory)
            .with_files_total_size_soft_limit(self.size_limit.as_u64());
        cache_config
    }
}

/// Wasmtime engine that is configured with linkers for all of the supported versions of
/// our pharia/skill WIT world.
pub struct Engine {
    inner: WasmtimeEngine,
}

impl Engine {
    /// How long to wait before incrementing the epoch counter.
    const EPOCH_INTERVAL: Duration = Duration::from_millis(100);
    /// Maximum skill execution time before we cancel the execution.
    /// Currently set to 10 minutes as an upper bound.
    const MAX_EXECUTION_TIME: Duration = Duration::from_secs(60 * 10);

    /// Creates a new engine instance.
    ///
    /// # Errors
    ///
    /// This function will return an error if the engine cannot be created,
    /// or if wasi functionality cannot be linked.
    pub fn new(config: EngineConfig) -> anyhow::Result<Self> {
        let engine = WasmtimeEngine::new(&config.try_into()?)?;

        // We only need a weak reference to pass to the loop.
        let engine_ref = engine.weak();

        // Increment epoch counter so that running skills have to yield
        // Uses a real thread to make sure this doesn't get blocked in
        // the async runtime by a skill that doesn't yield.
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Self::EPOCH_INTERVAL);
                // If the engine is still alive, increment the epoch counter.
                // Otherwise stop the thread.
                let Some(engine) = engine_ref.upgrade() else {
                    break;
                };
                engine.increment_epoch();
            }
        });

        Ok(Self { inner: engine })
    }

    /// Creates a new linker for the engine.
    /// This linker can be used to register functions and globals that can be called from WebAssembly code.
    /// It will already be linked with wasmtime-wasi for wasi implementations.
    ///
    /// `allow_shadowing` - Whether to allow shadowing of existing functions and globals. This is helpful
    /// if you are linking multiple worlds that might share some common interfaces and therefore would be
    /// defined twice and would error by default. You should only enable this if you are sure the two
    /// implementations are in fact identical, and therefore this is safe to do.
    ///
    /// # Errors
    ///
    /// Will error if the linker is unable to link the required WASI interfaces.
    pub fn new_linker<T: WasiView>(&self, allow_shadowing: bool) -> anyhow::Result<Linker<T>> {
        let mut linker = Linker::new(&self.inner);
        linker.allow_shadowing(allow_shadowing);
        // provide host implementation of WASI interfaces required by the component with wit-bindgen
        add_to_linker_async(&mut linker)?;
        Ok(linker)
    }

    /// Create a new component from this engine
    ///
    /// # Errors
    /// Returns an error if the component could not be created.
    pub fn new_component(&self, bytes: impl AsRef<[u8]>) -> anyhow::Result<Component> {
        Component::new(&self.inner, bytes)
    }

    /// Generates a store for a specific invocation.
    /// This will yield after every tick, as well as halt execution after `Self::MAX_EXECUTION_TIME`.
    pub fn store<Ctx>(&self, ctx: Ctx) -> Store<LinkerImpl<Ctx>>
    where
        Ctx: Send,
    {
        let ctx = LinkerImpl::new(ctx);
        let mut store = Store::new(&self.inner, ctx);
        // Check after the next tick
        store.set_epoch_deadline(1);
        // Once the deadline is reached, the callback will be called.
        // If the skill hasn't been running for more than 10 minutes, it will yield
        // and be allowed to run for one more tick.
        // If it has been running for more than 10 minutes, it will trap and return an error.
        let start = Instant::now();
        store.epoch_deadline_callback(move |_| {
            if start.elapsed() < Self::MAX_EXECUTION_TIME {
                Ok(UpdateDeadline::Yield(1))
            } else {
                Err(anyhow::anyhow!("Maximum skill execution time reached."))
            }
        });
        store
    }
}

/// Implementation for a given linker.
/// By default, it provides WASI support and a resource table.
/// But it is generic over a type for custom implementations of WIT interfaces.
pub struct LinkerImpl<Ctx> {
    pub ctx: Ctx,
    pub resource_table: ResourceTable,
    wasi_ctx: WasiCtx,
}

impl<Ctx> LinkerImpl<Ctx>
where
    Ctx: Send,
{
    pub fn new(ctx: Ctx) -> Self {
        LinkerImpl {
            ctx,
            resource_table: ResourceTable::new(),
            wasi_ctx: WasiCtx::builder().build(),
        }
    }
}

impl<Ctx> WasiView for LinkerImpl<Ctx>
where
    Ctx: Send,
{
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi_ctx
    }
}

impl<Ctx> IoView for LinkerImpl<Ctx>
where
    Ctx: Send,
{
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.resource_table
    }
}

/// Cranelift has the ability to cache compiled code for a given a wasm function.
/// This is based on the actual wasm bytecode, not necessarily per module, which means
/// that this cache can benefit modules that look very similar but came from different places.
///
/// This is beneficial to us because we are hosting similar types of components, that have
/// things like the same Python interpreter or SDK dependencies, which means we can reuse all
/// of this compilation across instances.
///
/// Based on benchmarks and tests, this can lead to a slight overhead (~5%) on the first compilation,
/// but then save 50-80% of the time for subsequent compilations.
///
/// This also only requires ~30MB of memory to store the cache, which seems like a small price to pay
/// for the benefits we gain in compile time.
///
/// We use a moka cache so that we can have an upper bound on the cache size, while also allowing for
/// automatic eviction of least recently used entries when the cache reaches its maximum capacity.
/// This hopefully optimizes for the most commonly used entries, as our skills are somewhat dynamic
/// in content.
///
/// Possible future improvements:
///
/// **Minimum cache entry size:**
/// Rustc and other compilers have a minimum size for cache entries, which it may be cheaper to regenerate
/// than to store in the cache. However, in benchmarking different minimum sizes, there wasn't a noticeable
/// difference in time saved, so for now we opt to just store everything.
///
/// **Cache crate**
/// Just using `DashMap` was faster, but it didn't provide a way to bound the cache size. There are other crates
/// that offer lighterweight versions than moka, but we use moka elsewhere and it is also one of the more
/// popular options and has a nicer API if you want to decide your upper bound without knowing the estimated
/// number of entries. The performance penalty seems reasonable for what we gain in terms of predictable
/// performance.
#[derive(Debug)]
struct IncrementalCompilationCache {
    /// Key: the precompiled bytes
    /// Value: the compiled bytes
    /// Based on Embark's implementation: <https://github.com/bytecodealliance/wasmtime/issues/4155#issuecomment-2767249113>
    /// but we use moka's `Cache` with a maximum size instead of an unbounded `DashMap`.
    cache: MokaCache<Vec<u8>, Vec<u8>>,
}

impl IncrementalCompilationCache {
    /// Create a new cache with the given maximum size.
    /// We use the stored bytes for key and value to determine the weight of each entry.
    fn new(max_cache_size: ByteSize) -> Self {
        Self {
            cache: MokaCache::builder()
                .weigher(|k: &Vec<u8>, v: &Vec<u8>| {
                    (k.len() + v.len()).try_into().unwrap_or(u32::MAX)
                })
                .max_capacity(max_cache_size.as_u64())
                .build(),
        }
    }
}

impl CacheStore for IncrementalCompilationCache {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.cache.get(key).map(Into::into)
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) -> bool {
        self.cache.insert(key.to_vec(), value);
        true
    }
}

/// The pooling allocator is tailor made for our use case, so
/// try to use it when we can. The main cost of the pooling allocator, however,
/// is the virtual memory required to run it. Not all systems support the same
/// amount of virtual memory, for example some aarch64 and riscv64 configuration
/// only support 39 bits of virtual address space.
///
/// The pooling allocator, by default, will request 1000 linear memories each
/// sized at 6G per linear memory. This is 6T of virtual memory which ends up
/// being about 42 bits of the address space. This exceeds the 39 bit limit of
/// some systems, so there the pooling allocator will fail by default.
///
/// This function attempts to dynamically determine the hint for the pooling
/// allocator. This returns `true` if the pooling allocator should be used
/// by default, or `false` otherwise.
///
/// The method for testing this is to allocate a 0-sized 64-bit linear memory
/// with a maximum size that's N bits large where we force all memories to be
/// static. This should attempt to acquire N bits of the virtual address space.
/// If successful that should mean that the pooling allocator is OK to use, but
/// if it fails then the pooling allocator is not used and the normal mmap-based
/// implementation is used instead.
///
/// Based on [`wasmtime serve`](https://github.com/bytecodealliance/wasmtime/blob/c42f925f3ab966e8446a807ea3cb59e3251aea5c/src/commands/serve.rs#L641) and [[`spin`](https://github.com/fermyon/spin/blob/2a9bf7c57eda9aa42152f016373d3105170b164b/crates/core/src/lib.rs#L157) implementations
fn pooling_allocator_is_supported() -> bool {
    const BITS_TO_TEST: u32 = 42;
    static USE_POOLING: LazyLock<bool> = LazyLock::new(|| {
        let mut config = Config::new();
        config.wasm_memory64(true);
        config.memory_reservation(1 << BITS_TO_TEST);
        let Ok(engine) = WasmtimeEngine::new(&config) else {
            info!(
                target: "pharia-kernel::skill-runtime",
                "unable to create an engine to test the pooling allocator, disabling pooling allocation"
            );
            return false;
        };
        let mut store = Store::new(&engine, ());
        // NB: the maximum size is in wasm pages to take out the 16-bits of wasm
        // page size here from the maximum size.
        let ty = MemoryType::new64(0, Some(1 << (BITS_TO_TEST - 16)));
        Memory::new(&mut store, ty).inspect_err(|_| {
            info!(
                target: "pharia-kernel::skill-runtime",
                "Pooling allocation not supported on this system. Falling back to mmap-based implementation."
            );
        }).is_ok()
    });
    *USE_POOLING
}

#[cfg(test)]
mod tests {
    use bytesize::ByteSize;
    use test_skills::given_python_skill_greet_v0_3;

    use super::*;

    #[test]
    fn file_cache() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let wasmtime_cache = WasmtimeCache::new(temp_dir.path(), ByteSize::mib(512));
        let engine_config = EngineConfig::default().with_wasmtime_cache(Some(wasmtime_cache));
        let bytes = given_python_skill_greet_v0_3().bytes();
        let engine = WasmtimeEngine::new(&engine_config.try_into()?)?;
        Component::new(&engine, &bytes)?;

        assert!(std::fs::read_dir(temp_dir.path())?.count() > 0);

        Ok(())
    }
}
