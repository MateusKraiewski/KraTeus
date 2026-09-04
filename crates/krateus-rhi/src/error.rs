//! Erros da RHI.
//!
//! As variantes descrevem **violacoes de contrato** — identificador invalido,
//! ordem errada de gravacao, descritor inconsistente — e nao falhas de driver.
//! Um backend real vai precisar de mais; quando precisar, a variante entra.

use crate::types::TextureFormat;

/// O que pode dar errado numa chamada de RHI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum RhiError {
    /// Identificador que nao corresponde a nenhum recurso vivo.
    ///
    /// Cobre tanto o handle nunca criado quanto o de recurso ja destruido — a
    /// geracao do handle distingue os dois casos internamente, e para quem
    /// chama a consequencia e a mesma.
    #[error("{tipo} invalido ou ja destruido")]
    IdentificadorInvalido {
        /// Que espécie de recurso era esperada.
        tipo: &'static str,
    },

    /// Descritor que nao descreve um recurso construivel.
    #[error("descritor invalido para {tipo}: {motivo}")]
    DescritorInvalido {
        /// Recurso que se tentava criar.
        tipo: &'static str,
        /// O que estava errado.
        motivo: &'static str,
    },

    /// Operacao pedida fora da ordem que a gravacao exige.
    #[error("ordem invalida: {0}")]
    OrdemInvalida(&'static str),

    /// Superficie usada antes de ser configurada.
    #[error("a superficie ainda nao foi configurada")]
    SuperficieNaoConfigurada,

    /// Formato pedido que a superficie nao aceita.
    #[error("formato {formato:?} nao suportado pela superficie")]
    FormatoNaoSuportado {
        /// Formato pedido.
        formato: TextureFormat,
    },

    /// Escrita que nao cabe no buffer.
    #[error("escrita de {bytes} bytes em {deslocamento} excede o buffer de {tamanho} bytes")]
    ForaDosLimites {
        /// Quantos bytes se tentou escrever.
        bytes: u64,
        /// A partir de onde.
        deslocamento: u64,
        /// Tamanho do buffer.
        tamanho: u64,
    },
}

/// Alias de `Result` com [`RhiError`] como erro padrao.
pub type Result<T, E = RhiError> = std::result::Result<T, E>;
