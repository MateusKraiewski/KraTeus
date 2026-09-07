//! `krateus-physics` — simulacao fisica.
//!
//! A secao 6 do documento de visao pede um subsistema reutilizavel capaz de
//! calcular movimento, forcas e trajetorias **sem depender de animacao
//! pre-programada**. Esta crate comeca por aí.
//!
//! # Estado
//!
//! Inicio da Fase 5, feito enquanto a Fase 3 aguarda ambiente grafico. O que
//! existe e CPU pura e verificavel:
//!
//! - [`trajetoria`] — a ferramenta de balistica do §6, com niveis de fidelidade
//! - [`integrador`] — integrador semi-implicito com gravidade, forcas e impulsos
//! - [`forma`] — formas de colisao e o volume envolvente que a broadphase indexa
//!
//! Faltam broadphase, narrowphase, resolucao de contato, raycast, character
//! controller e os gizmos de debug. Os gizmos dependem do renderer; o resto,
//! nao.
//!
//! # Determinismo
//!
//! A solucao de balistica usa apenas raiz quadrada e aritmetica, evitando as
//! funcoes transcendentais da libm, que sao onde plataformas divergem (D09). O
//! integrador percorre entidades na ordem estavel do ECS.

pub mod forma;
pub mod integrador;
pub mod trajetoria;

pub use forma::{Aabb, Forma};
pub use integrador::{
    AcumuladorDeForca, Gravidade, MassaInversa, Posicao, Velocidade, aplicar_impulso,
    integrar_um_passo, sistema_integrar,
};
pub use trajetoria::{Fidelidade, G_TERRA, Lancamento, Salto, SaltoImpossivel};
