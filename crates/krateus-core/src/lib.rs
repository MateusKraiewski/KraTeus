//! `krateus-core` — fundacao compartilhada da KraTeus Engine.
//!
//! Esta crate e folha no grafo de dependencias: nao depende de nenhum outro
//! subsistema da engine, e essa propriedade e verificada no CI. Tudo que ela
//! oferece precisa fazer sentido para simulacao, render, fisica e ferramentas
//! ao mesmo tempo — se um tipo so serve a um subsistema, ele pertence a esse
//! subsistema, nao aqui.
//!
//! Ver `docs/ROADMAP.md`, Fase 1.

pub mod config;
pub mod error;
pub mod handle;
pub mod log;
pub mod pool;
pub mod time;

/// Reexporta o que praticamente todo consumidor da engine usa.
pub mod prelude {
    pub use crate::error::{Error, Result};
    pub use crate::handle::Handle;
    pub use crate::pool::Pool;
    pub use crate::time::{Clock, Time};
}
