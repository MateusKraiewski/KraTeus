//! Baseline do ECS. Alimenta os criterios de aceite da Fase 2.
//!
//! Executar com: `cargo bench -p krateus-ecs --bench ecs --features bench`
//!
//! Nao roda na maquina de desenvolvimento enquanto o R01 estiver aberto — o
//! Smart App Control bloqueia o build script de `num-traits`. A medicao vive no
//! CI Linux. Ver `docs/RISCOS.md`.

// `criterion_group!` expande para uma funcao publica sem documentacao.
#![allow(missing_docs)]

use criterion::{Criterion, criterion_group, criterion_main};
use krateus_ecs::{With, World};
use std::hint::black_box;

struct Posicao(f32, f32, f32);
struct Velocidade(f32, f32, f32);
struct Aceleracao(f32, f32, f32);
struct Estatica;

fn mundo_com(n: u32) -> World {
    let mut w = World::new();
    for i in 0..n {
        let f = i as f32;
        w.spawn((Posicao(f, f, f), Velocidade(1.0, 1.0, 1.0), Aceleracao(0.1, 0.1, 0.1)));
    }
    w
}

fn spawn(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("ecs/spawn");
    grupo.sample_size(10);
    grupo.bench_function("100k_tres_componentes", |b| {
        b.iter(|| black_box(mundo_com(100_000)));
    });
    grupo.finish();
}

fn spawn_despawn(c: &mut Criterion) {
    // Regime permanente: o custo deve estabilizar, sem crescer com o tempo.
    c.bench_function("ecs/ciclo_spawn_despawn_10k", |b| {
        let mut w = World::new();
        b.iter(|| {
            for i in 0..10_000 {
                let f = i as f32;
                let e = w.spawn((Posicao(f, f, f), Velocidade(1.0, 1.0, 1.0)));
                black_box(w.despawn(e));
            }
        });
    });
}

fn iteracao(c: &mut Criterion) {
    let mut grupo = c.benchmark_group("ecs/query");
    grupo.sample_size(10);

    // O criterio de aceite da Fase 2: 1M de entidades com tres componentes.
    let mut w = mundo_com(1_000_000);
    grupo.bench_function("1M_tres_componentes", |b| {
        b.iter(|| {
            for (p, v, a) in w.query::<(&mut Posicao, &mut Velocidade, &Aceleracao)>() {
                v.0 += a.0;
                v.1 += a.1;
                v.2 += a.2;
                p.0 += v.0;
                p.1 += v.1;
                p.2 += v.2;
            }
        });
    });

    grupo.bench_function("1M_somente_leitura", |b| {
        b.iter(|| {
            let mut soma = 0.0_f32;
            for p in w.query::<&Posicao>() {
                soma += p.0;
            }
            black_box(soma)
        });
    });

    grupo.finish();
}

fn iteracao_fragmentada(c: &mut Criterion) {
    // Metade das entidades em cada archetype: mede o custo de atravessar mais
    // de um archetype, que e o caso real de qualquer jogo.
    let mut w = World::new();
    for i in 0..500_000 {
        let f = i as f32;
        w.spawn((Posicao(f, f, f), Velocidade(1.0, 1.0, 1.0)));
        w.spawn((Posicao(f, f, f), Velocidade(1.0, 1.0, 1.0), Estatica));
    }

    let mut grupo = c.benchmark_group("ecs/query_fragmentada");
    grupo.sample_size(10);

    grupo.bench_function("1M_em_dois_archetypes", |b| {
        b.iter(|| {
            for (p, v) in w.query::<(&mut Posicao, &Velocidade)>() {
                p.0 += v.0;
            }
        });
    });

    grupo.bench_function("500k_com_filtro_without", |b| {
        b.iter(|| {
            let mut n = 0_u32;
            for _ in w.query_filtered::<&Posicao, krateus_ecs::Without<Estatica>>() {
                n += 1;
            }
            black_box(n)
        });
    });

    grupo.bench_function("500k_com_filtro_with", |b| {
        b.iter(|| {
            let mut n = 0_u32;
            for _ in w.query_filtered::<&Posicao, With<Estatica>>() {
                n += 1;
            }
            black_box(n)
        });
    });

    grupo.finish();
}

criterion_group!(benches, spawn, spawn_despawn, iteracao, iteracao_fragmentada);
criterion_main!(benches);
