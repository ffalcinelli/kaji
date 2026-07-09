use criterion::{Criterion, black_box, criterion_group, criterion_main};
use kaji::models::ToOptionString;

fn bench_to_option_string(c: &mut Criterion) {
    let s = String::from(
        "a_very_long_string_that_should_not_be_cloned_if_we_can_help_it_even_though_its_just_a_string_it_costs_some_cpu_cycles",
    );

    c.bench_function("String::to_option_string", |b| {
        b.iter(|| black_box(&s).to_option_string());
    });

    let opt_s = Some(String::from(
        "a_very_long_string_that_should_not_be_cloned_if_we_can_help_it_even_though_its_just_a_string_it_costs_some_cpu_cycles",
    ));
    c.bench_function("Option<String>::to_option_string", |b| {
        b.iter(|| black_box(&opt_s).to_option_string());
    });
}

criterion_group!(benches, bench_to_option_string);
criterion_main!(benches);
