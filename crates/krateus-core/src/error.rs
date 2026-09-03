//! Tipo de erro compartilhado pelo core.

use std::path::PathBuf;

/// Erros originados no core da engine.
///
/// Cada variante carrega o caminho envolvido: um erro de configuracao sem
/// dizer qual arquivo falhou custa mais tempo de diagnostico do que o erro em
/// si.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Falha de leitura ou escrita em disco.
    #[error("falha de I/O em {}", path.display())]
    Io {
        /// Caminho que causou a falha.
        path: PathBuf,
        /// Erro original do sistema de arquivos.
        #[source]
        source: std::io::Error,
    },

    /// Arquivo de configuracao malformado ou incompativel com o tipo alvo.
    #[error("configuracao invalida em {}", path.display())]
    ConfigParse {
        /// Caminho do arquivo de configuracao.
        path: PathBuf,
        /// Erro original do parser TOML.
        #[source]
        source: toml::de::Error,
    },

    /// Falha ao reserializar a configuracao durante a fusao de camadas.
    #[error("falha ao serializar configuracao")]
    ConfigEncode(#[from] toml::ser::Error),
}

/// Alias de `Result` com o erro do core como padrao.
pub type Result<T, E = Error> = std::result::Result<T, E>;
