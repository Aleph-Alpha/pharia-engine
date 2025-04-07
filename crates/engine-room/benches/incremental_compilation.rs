fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[divan::bench_group]
mod compilation_time {
    use bytesize::ByteSize;
    use divan::{Bencher, counter::BytesCount};
    use engine_room::Engine;
    use test_skills::{given_python_skill_greet_v0_3, given_rust_skill_greet_v0_3};

    const MAX_CACHE_SIZE: &[Option<ByteSize>] = &[
        None,
        Some(ByteSize::mib(32)),
        Some(ByteSize::mib(64)),
        Some(ByteSize::mib(128)),
        Some(ByteSize::mib(256)),
        Some(ByteSize::mib(512)),
    ];
    const NUM_SKILLS: &[usize] = &[1, 2, 3];

    #[divan::bench(sample_count = 25, args=MAX_CACHE_SIZE, consts=NUM_SKILLS)]
    fn python<const N: usize>(bencher: Bencher<'_, '_>, max_cache_size: Option<ByteSize>) {
        bench::<N>(
            bencher,
            || given_python_skill_greet_v0_3().bytes(),
            max_cache_size,
        );
    }

    #[divan::bench(sample_count = 25, args=MAX_CACHE_SIZE, consts=NUM_SKILLS)]
    fn rust<const N: usize>(bencher: Bencher<'_, '_>, max_cache_size: Option<ByteSize>) {
        bench::<N>(
            bencher,
            || given_rust_skill_greet_v0_3().bytes(),
            max_cache_size,
        );
    }

    /// Shared benchmark setup
    /// Benchmarks creation of a component. Doesn't include instantiation/linking
    fn bench<const N: usize>(
        bencher: Bencher<'_, '_>,
        skill: impl Fn() -> Vec<u8> + Sync,
        max_cache_size: Option<ByteSize>,
    ) {
        bencher
            .with_inputs(|| (Engine::new(false, max_cache_size).unwrap(), skill()))
            .input_counter(|(_, bytes)| BytesCount::new(bytes.len() * N))
            .bench_values(|(engine, bytes)| {
                for _ in 0..N {
                    engine.new_component(&bytes).unwrap();
                }
            });
    }
}
