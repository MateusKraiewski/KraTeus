//! Carregamento de configuracao em camadas.
//!
//! Um arquivo base versionado no projeto, sobreposto por um arquivo local
//! opcional apontado por `KRATEUS_CONFIG`. A fusao e recursiva: a camada de
//! cima substitui apenas as chaves que declara, e nao o bloco inteiro em que
//! elas estao.
//!
//! Isso existe para que ajustar uma opcao de qualidade grafica na maquina de um
//! desenvolvedor nao exija editar — e arriscar comitar — o arquivo do projeto.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use toml::Value;

use crate::error::{Error, Result};

/// Variavel de ambiente com o caminho do arquivo de sobreposicao.
pub const ENV_OVERRIDE: &str = "KRATEUS_CONFIG";

/// Carrega a configuracao de `path`, aplicando a sobreposicao de
/// [`ENV_OVERRIDE`] quando ela existir.
///
/// # Errors
///
/// Se algum dos arquivos nao puder ser lido, nao for TOML valido, ou o
/// resultado da fusao nao corresponder ao tipo `T`.
pub fn load<T: DeserializeOwned>(path: impl AsRef<Path>) -> Result<T> {
    let path = path.as_ref();
    let mut base = ler_toml(path)?;

    if let Some(sobreposicao) = std::env::var_os(ENV_OVERRIDE) {
        let sobreposicao = PathBuf::from(sobreposicao);
        tracing::info!(path = %sobreposicao.display(), "aplicando sobreposicao de configuracao");
        merge(&mut base, ler_toml(&sobreposicao)?);
    }

    from_value(base, path)
}

/// Desserializa a configuracao a partir de uma string TOML, sem tocar em disco.
///
/// # Errors
///
/// Se o texto nao for TOML valido ou nao corresponder ao tipo `T`.
pub fn load_str<T: DeserializeOwned>(texto: &str) -> Result<T> {
    toml::from_str(texto)
        .map_err(|source| Error::ConfigParse { path: PathBuf::from("<memoria>"), source })
}

fn ler_toml(path: &Path) -> Result<Value> {
    let texto = std::fs::read_to_string(path)
        .map_err(|source| Error::Io { path: path.to_path_buf(), source })?;

    toml::from_str(&texto).map_err(|source| Error::ConfigParse { path: path.to_path_buf(), source })
}

fn from_value<T: DeserializeOwned>(valor: Value, path: &Path) -> Result<T> {
    let texto = toml::to_string(&valor)?;
    toml::from_str(&texto).map_err(|source| Error::ConfigParse { path: path.to_path_buf(), source })
}

/// Funde `sobreposicao` sobre `base`, recursivamente em tabelas.
///
/// Valores escalares e arrays sao substituidos por inteiro: fundir arrays
/// elemento a elemento produz resultados que ninguem consegue prever lendo os
/// dois arquivos.
fn merge(base: &mut Value, sobreposicao: Value) {
    match sobreposicao {
        Value::Table(tabela) => {
            let Value::Table(alvo) = base else {
                *base = Value::Table(tabela);
                return;
            };
            for (chave, valor) in tabela {
                match alvo.get_mut(&chave) {
                    Some(existente) => merge(existente, valor),
                    None => {
                        alvo.insert(chave, valor);
                    }
                }
            }
        }
        outro => *base = outro,
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize, PartialEq)]
    struct Render {
        backend: String,
        vsync: bool,
        sombras: u32,
    }

    #[derive(Debug, Deserialize, PartialEq)]
    struct Cfg {
        nome: String,
        render: Render,
    }

    const BASE: &str = r#"
        nome = "krateus"
        [render]
        backend = "wgpu"
        vsync = true
        sombras = 4
    "#;

    #[test]
    fn desserializa_o_arquivo_base() {
        let cfg: Cfg = load_str(BASE).unwrap();
        assert_eq!(cfg.nome, "krateus");
        assert_eq!(cfg.render.sombras, 4);
    }

    #[test]
    fn sobreposicao_preserva_chaves_nao_declaradas() {
        let mut base: Value = toml::from_str(BASE).unwrap();
        let over: Value = toml::from_str("[render]\nsombras = 0\n").unwrap();

        merge(&mut base, over);
        let cfg: Cfg = from_value(base, Path::new("<teste>")).unwrap();

        // So `sombras` mudou; o bloco `[render]` nao foi substituido inteiro.
        assert_eq!(cfg.render.sombras, 0);
        assert_eq!(cfg.render.backend, "wgpu");
        assert!(cfg.render.vsync);
    }

    #[test]
    fn sobreposicao_substitui_escalar_de_tipo_diferente() {
        let mut base: Value = toml::from_str("x = 1").unwrap();
        merge(&mut base, toml::from_str(r#"x = "texto""#).unwrap());
        assert_eq!(base["x"].as_str(), Some("texto"));
    }

    #[test]
    fn erro_de_io_diz_qual_arquivo_falhou() {
        let err = load::<Cfg>("nao/existe/krateus.toml").unwrap_err();
        assert!(err.to_string().contains("nao/existe/krateus.toml"), "mensagem: {err}");
    }
}
