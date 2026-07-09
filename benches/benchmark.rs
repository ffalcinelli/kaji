use criterion::{Criterion, criterion_group, criterion_main};
use kaji::validate;
use std::path::PathBuf;

fn criterion_benchmark(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let workspace_dir = PathBuf::from("dummy_workspace");

    c.bench_function("validate_realm", |b| {
        b.to_async(&runtime)
            .iter(|| validate::run(workspace_dir.clone(), &[]));
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
