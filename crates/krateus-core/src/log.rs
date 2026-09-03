//! Inicializacao do logging estruturado.
//!
//! A engine usa `tracing` em vez de um logger de linhas porque as spans sao a
//! mesma estrutura que o profiler de CPU da Fase 7 vai consumir: instrumentar
//! uma vez serve para diagnostico e para medicao.
//!
//! O filtro vem de `KRATEUS_LOG`, com a sintaxe de `RUST_LOG`. Isso permite
//! silenciar ou detalhar subsistemas individualmente sem recompilar:
//!
//! ```text
//! KRATEUS_LOG=info,krateus_physics=trace,krateus_render=warn
//! ```

use tracing_subscriber::EnvFilter;

/// Variavel de ambiente que controla o filtro de log.
pub const ENV_FILTRO: &str = "KRATEUS_LOG";

/// Filtro aplicado quando [`ENV_FILTRO`] nao esta definida.
pub const FILTRO_PADRAO: &str = "info";

/// Inicializa o subscriber global de logging.
///
/// Idempotente: chamar mais de uma vez e inofensivo, e chamadas posteriores nao
/// tem efeito. Devolve `true` se esta chamada foi a que instalou o subscriber,
/// o que permite ao editor e ao runtime chamarem sem coordenacao entre si.
pub fn init() -> bool {
    init_with(FILTRO_PADRAO)
}

/// Como [`init`], mas com um filtro padrao explicito.
pub fn init_with(filtro_padrao: &str) -> bool {
    let filtro = EnvFilter::try_from_env(ENV_FILTRO)
        .or_else(|_| EnvFilter::try_new(filtro_padrao))
        .unwrap_or_else(|_| EnvFilter::new(FILTRO_PADRAO));

    tracing_subscriber::fmt()
        .with_env_filter(filtro)
        .with_target(true)
        .with_level(true)
        .try_init()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_e_idempotente() {
        // A primeira chamada pode ou nao vencer a corrida com outros testes do
        // mesmo binario; o que importa e que a segunda nunca entre em panico.
        let _ = init();
        assert!(!init(), "a segunda inicializacao nao deve reinstalar o subscriber");
    }
}
