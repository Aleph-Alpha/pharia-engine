fn main() {
    // Run registered benchmarks.
    divan::main();
}

#[divan::bench_group]
mod compilation_time {
    use divan::{Bencher, counter::BytesCount};
    use engine_room::Engine;
    use test_skills::{given_python_skill_greet_v0_3, given_rust_skill_greet_v0_3};

    /// Shared benchmark setup
    /// Benchmarks creation of a component. Doesn't include instantiation/linking
    fn bench(bencher: Bencher<'_, '_>, skill: impl Fn() -> Vec<u8> + Sync) {
        bencher
            .with_inputs(|| (Engine::new(false).unwrap(), skill()))
            .input_counter(|(_, bytes)| BytesCount::of_slice(bytes))
            .bench_values(|(engine, bytes)| {
                engine.new_component(bytes).unwrap();
            });
    }

    #[divan::bench(sample_count = 10)]
    fn python(bencher: Bencher<'_, '_>) {
        bench(bencher, || given_python_skill_greet_v0_3().bytes());
    }

    #[divan::bench]
    fn rust(bencher: Bencher<'_, '_>) {
        bench(bencher, || given_rust_skill_greet_v0_3().bytes());
    }
}
