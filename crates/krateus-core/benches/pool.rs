//! Baseline do `Pool`. Existe para que a Fase 2 tenha com o que comparar.
//!
//! Executar com: `cargo bench -p krateus-core`

use criterion::{Criterion, criterion_group, criterion_main};
use krateus_core::prelude::*;
use std::hint::black_box;

#[derive(Clone, Copy)]
struct Corpo {
    posicao: [f32; 3],
    velocidade: [f32; 3],
}

impl Corpo {
    const ZERO: Self = Self { posicao: [0.0; 3], velocidade: [1.0; 3] };
}

fn insercao(c: &mut Criterion) {
    c.bench_function("pool/insert_100k", |b| {
        b.iter(|| {
            let mut pool = Pool::with_capacity(100_000);
            for _ in 0..100_000 {
                pool.insert(black_box(Corpo::ZERO));
            }
            pool
        });
    });
}

fn ciclo_estavel(c: &mut Criterion) {
    // O caso que justifica a existencia do pool: criacao e destruicao em regime
    // permanente nao deve alocar depois do aquecimento.
    c.bench_function("pool/ciclo_insert_remove_10k", |b| {
        let mut pool: Pool<Corpo> = Pool::with_capacity(1024);
        b.iter(|| {
            for _ in 0..10_000 {
                let h = pool.insert(black_box(Corpo::ZERO));
                black_box(pool.remove(h));
            }
        });
    });
}

fn iteracao(c: &mut Criterion) {
    let mut pool = Pool::with_capacity(100_000);
    for _ in 0..100_000 {
        pool.insert(Corpo::ZERO);
    }

    c.bench_function("pool/iter_mut_100k", |b| {
        b.iter(|| {
            for (_, corpo) in pool.iter_mut() {
                corpo.posicao[0] += corpo.velocidade[0];
            }
            black_box(&pool);
        });
    });
}

criterion_group!(benches, insercao, ciclo_estavel, iteracao);
criterion_main!(benches);
