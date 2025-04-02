//! The Pharia Kernel Engine Room
//!
//! This library hopefully contains and abstracts all of the core wasmtime knowledge needed to run
//! skills within the Pharia Kernel. There will still be bindings and linking that happens outside
//! of this library, but everything related to efficient execution of wasm components, regardless
//! of the world it targets, should live here.

use std::{
    sync::LazyLock,
    time::{Duration, Instant},
};

use tracing::info;
use wasmtime::{
    Config, Engine as WasmtimeEngine, InstanceAllocationStrategy, Memory, MemoryType, OptLevel,
    Store, UpdateDeadline,
    component::{Component, Linker},
};
use wasmtime_wasi::{IoView, ResourceTable, WasiCtx, WasiView};

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
    pub fn new(use_pooling_allocator: bool) -> anyhow::Result<Self> {
        let mut config = Config::new();
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

        let engine = WasmtimeEngine::new(&config)?;

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
        wasmtime_wasi::add_to_linker_async(&mut linker)?;
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
                "unable to create an engine to test the pooling allocator, disabling pooling allocation"
            );
            return false;
        };
        let mut store = Store::new(&engine, ());
        // NB: the maximum size is in wasm pages to take out the 16-bits of wasm
        // page size here from the maximum size.
        let ty = MemoryType::new64(0, Some(1 << (BITS_TO_TEST - 16)));
        Memory::new(&mut store, ty).inspect_err(|_| {
            info!("Pooling allocation not supported on this system. Falling back to mmap-based implementation.");
        }).is_ok()
    });
    *USE_POOLING
}
