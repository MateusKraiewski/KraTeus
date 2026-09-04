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
use krateus_core::jobs::JobPool;
use krateus_ecs::{Query, Schedule, With, World};
use std::hint::black_box;

struct Posicao(f32, f32, f32);
struct Velocidade(f32, f32, f32);
struct Aceleracao(f32, f32, f32);
struct Estatica;
struct Idade(u32);
struct Calor(f32);

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

/// Escalabilidade do scheduler de 1 a N threads.
///
/// Criterio de aceite da Fase 2. Os quatro sistemas escrevem componentes
/// distintos, entao o `Access` os coloca todos na mesma subetapa e a curva
/// mostra o ganho real do paralelismo.
fn escalabilidade_do_scheduler(c: &mut Criterion) {
    fn integrar(mut q: Query<(&mut Posicao, &Velocidade)>) {
        for (p, v) in q.iter() {
            p.0 += v.0;
            p.1 += v.1;
            p.2 += v.2;
        }
    }
    fn acelerar(mut q: Query<(&mut Velocidade, &Aceleracao)>) {
        for (v, a) in q.iter() {
            v.0 += a.0;
        }
    }
    fn envelhecer(mut q: Query<&mut Idade>) {
        for i in q.iter() {
            i.0 += 1;
        }
    }
    fn aquecer(mut q: Query<&mut Calor>) {
        for t in q.iter() {
            t.0 *= 0.99;
        }
    }

    let mut grupo = c.benchmark_group("ecs/scheduler");
    grupo.sample_size(10);

    for threads in [0_usize, 1, 2, 4] {
        let mut w = World::new();
        for i in 0..200_000_u32 {
            let f = i as f32;
            w.spawn((
                Posicao(f, f, f),
                Velocidade(1.0, 1.0, 1.0),
                Aceleracao(0.01, 0.01, 0.01),
                Idade(0),
                Calor(1.0),
            ));
        }

        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", integrar);
        s.add_system("simulacao", acelerar);
        s.add_system("simulacao", envelhecer);
        s.add_system("simulacao", aquecer);
        s.initialize(&mut w);

        // Integrar le Velocidade e acelerar a escreve: duas subetapas, com
        // envelhecer e aquecer paralelizando junto.
        assert_eq!(s.batches("simulacao").map(<[Vec<usize>]>::len), Some(2));

        let pool = JobPool::new(threads);
        grupo.bench_function(format!("200k_entidades_{threads}_threads_extras"), |b| {
            b.iter(|| {
                s.run_parallel(&mut w, &pool);
                black_box(&w);
            });
        });
    }

    grupo.finish();
}

criterion_group!(
    benches,
    spawn,
    spawn_despawn,
    iteracao,
    iteracao_fragmentada,
    escalabilidade_do_scheduler
);
criterion_main!(benches);
