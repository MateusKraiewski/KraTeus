//! Criterio de aceite da Fase 1: logging, configuracao em camadas, pool com
//! handles e 10.000 passos de simulacao com timestep fixo.
//!
//! Executar com: `cargo run -p krateus-core --example hello_core`

use std::time::Duration;

use krateus_core::prelude::*;
use krateus_core::{config, log};
use serde::Deserialize;

/// Configuracao de exemplo, no formato que a engine vai usar.
#[derive(Debug, Deserialize)]
struct Config {
    simulacao: Simulacao,
}

#[derive(Debug, Deserialize)]
struct Simulacao {
    ticks_por_segundo: u32,
    passos: u64,
}

const CONFIG_PADRAO: &str = "
[simulacao]
ticks_por_segundo = 60
passos = 10000
";

/// Uma entidade de brincadeira, so para exercitar o pool.
#[derive(Debug)]
struct Corpo {
    posicao: f32,
    velocidade: f32,
}

fn main() -> Result<()> {
    log::init();

    let cfg: Config = config::load_str(CONFIG_PADRAO)?;
    tracing::info!(?cfg.simulacao, "configuracao carregada");

    let mut clock = Clock::new(cfg.simulacao.ticks_por_segundo);
    let mut corpos: Pool<Corpo> = Pool::with_capacity(1024);

    let handles: Vec<Handle<Corpo>> = (0..1024)
        .map(|i| corpos.insert(Corpo { posicao: 0.0, velocidade: i as f32 * 0.01 }))
        .collect();

    tracing::info!(corpos = corpos.len(), "mundo povoado");

    // Alimenta o relogio com um frame de duracao irregular, como aconteceria de
    // verdade, e roda ate fechar o numero de passos pedido.
    let mut frame = 0_u64;
    while clock.tick_count() < cfg.simulacao.passos {
        frame += 1;
        let real = Duration::from_micros(15_000 + (frame % 7) * 900);
        clock.accumulate(real);

        while let Some(time) = clock.next_tick() {
            if clock.tick_count() > cfg.simulacao.passos {
                break;
            }
            for (_h, corpo) in corpos.iter_mut() {
                corpo.posicao += corpo.velocidade * time.delta_seconds();
            }
        }
    }

    // Um handle liberado nunca volta a resolver, mesmo com o slot reciclado.
    let removido = handles[0];
    corpos.remove(removido);
    assert!(!corpos.contains(removido));

    let soma: f32 = corpos.iter().map(|(_, c)| c.posicao).sum();
    tracing::info!(
        passos = clock.tick_count(),
        frames = frame,
        tempo_simulado_s = clock.elapsed().as_secs_f32(),
        alpha = clock.alpha(),
        soma_posicoes = soma,
        "simulacao concluida"
    );

    Ok(())
}
