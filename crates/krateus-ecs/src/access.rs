//! Conjunto de componentes lidos e escritos por uma query ou sistema.
//!
//! Serve a dois propositos, e o segundo e a razao de este tipo existir tao
//! cedo:
//!
//! 1. Rejeitar, na criacao, uma query que conflitaria consigo mesma — como
//!    `(&mut Posicao, &mut Posicao)`, que entregaria duas referencias mutaveis
//!    ao mesmo valor. E a diferenca entre um `panic` claro e comportamento
//!    indefinido.
//! 2. Alimentar o scheduler da Fase 2. Sistemas cujos acessos nao colidem podem
//!    rodar em paralelo, e essa decisao sai daqui — derivada dos tipos, sem que
//!    ninguem precise ordenar sistemas a mao (secao 5 do documento de visao).

use crate::component::ComponentId;

/// Componentes lidos e escritos.
///
/// As listas ficam ordenadas e sem repeticao, o que torna as comparacoes uma
/// intersecao linear em vez de um produto cartesiano.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Access {
    leituras: Vec<ComponentId>,
    escritas: Vec<ComponentId>,
    /// Registra que algum componente foi pedido para escrita mais de uma vez.
    /// Guardado a parte porque `escritas` e desduplicada.
    escrita_duplicada: Option<ComponentId>,
}

impl Access {
    /// Cria um acesso vazio.
    #[must_use]
    pub const fn new() -> Self {
        Self { leituras: Vec::new(), escritas: Vec::new(), escrita_duplicada: None }
    }

    /// Registra leitura de `id`.
    pub fn add_read(&mut self, id: ComponentId) {
        insere_ordenado(&mut self.leituras, id);
    }

    /// Registra escrita de `id`.
    pub fn add_write(&mut self, id: ComponentId) {
        if !insere_ordenado(&mut self.escritas, id) {
            self.escrita_duplicada.get_or_insert(id);
        }
    }

    /// Componentes lidos, ordenados.
    #[inline]
    #[must_use]
    pub fn reads(&self) -> &[ComponentId] {
        &self.leituras
    }

    /// Componentes escritos, ordenados.
    #[inline]
    #[must_use]
    pub fn writes(&self) -> &[ComponentId] {
        &self.escritas
    }

    /// Indica se o acesso nao toca em nada.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.leituras.is_empty() && self.escritas.is_empty()
    }

    /// Componente que este acesso pede de forma incompativel consigo mesmo.
    ///
    /// Acontece quando o mesmo componente e escrito duas vezes, ou lido e
    /// escrito ao mesmo tempo. Devolve o primeiro encontrado.
    #[must_use]
    pub fn self_conflict(&self) -> Option<ComponentId> {
        self.escrita_duplicada.or_else(|| primeira_intersecao(&self.leituras, &self.escritas))
    }

    /// Componente que impede este acesso de rodar em paralelo com `outro`.
    ///
    /// Duas leituras nunca colidem. Qualquer escrita colide com leitura ou
    /// escrita do mesmo componente.
    #[must_use]
    pub fn conflict_with(&self, outro: &Self) -> Option<ComponentId> {
        primeira_intersecao(&self.escritas, &outro.escritas)
            .or_else(|| primeira_intersecao(&self.escritas, &outro.leituras))
            .or_else(|| primeira_intersecao(&self.leituras, &outro.escritas))
    }

    /// Indica se os dois acessos podem rodar ao mesmo tempo.
    #[inline]
    #[must_use]
    pub fn compatible_with(&self, outro: &Self) -> bool {
        self.conflict_with(outro).is_none()
    }
}

/// Insere mantendo a ordem. Devolve `false` se ja estava presente.
fn insere_ordenado(v: &mut Vec<ComponentId>, id: ComponentId) -> bool {
    match v.binary_search(&id) {
        Ok(_) => false,
        Err(pos) => {
            v.insert(pos, id);
            true
        }
    }
}

/// Primeiro elemento comum a duas listas ordenadas.
fn primeira_intersecao(a: &[ComponentId], b: &[ComponentId]) -> Option<ComponentId> {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => return Some(a[i]),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::Components;

    fn ids() -> (ComponentId, ComponentId, ComponentId) {
        let mut c = Components::new();
        (c.register::<u8>(), c.register::<u16>(), c.register::<u32>())
    }

    #[test]
    fn acesso_vazio_nao_conflita() {
        let a = Access::new();
        assert!(a.is_empty());
        assert_eq!(a.self_conflict(), None);
        assert!(a.compatible_with(&Access::new()));
    }

    #[test]
    fn leituras_nunca_colidem_entre_si() {
        let (x, y, _) = ids();
        let mut a = Access::new();
        a.add_read(x);
        a.add_read(y);

        let mut b = Access::new();
        b.add_read(x);

        assert!(a.compatible_with(&b));
    }

    #[test]
    fn escrita_colide_com_leitura_do_mesmo_componente() {
        let (x, _, _) = ids();
        let mut escritor = Access::new();
        escritor.add_write(x);

        let mut leitor = Access::new();
        leitor.add_read(x);

        assert_eq!(escritor.conflict_with(&leitor), Some(x));
        assert_eq!(leitor.conflict_with(&escritor), Some(x));
    }

    #[test]
    fn escritas_de_componentes_distintos_sao_paralelizaveis() {
        let (x, y, _) = ids();
        let mut a = Access::new();
        a.add_write(x);
        let mut b = Access::new();
        b.add_write(y);

        assert!(a.compatible_with(&b));
    }

    #[test]
    fn escrever_duas_vezes_o_mesmo_componente_e_autoconflito() {
        let (x, _, _) = ids();
        let mut a = Access::new();
        a.add_write(x);
        a.add_write(x);

        assert_eq!(a.self_conflict(), Some(x));
    }

    #[test]
    fn ler_e_escrever_o_mesmo_componente_e_autoconflito() {
        let (x, _, _) = ids();
        let mut a = Access::new();
        a.add_read(x);
        a.add_write(x);

        assert_eq!(a.self_conflict(), Some(x));
    }

    #[test]
    fn ler_o_mesmo_componente_duas_vezes_e_permitido() {
        let (x, _, _) = ids();
        let mut a = Access::new();
        a.add_read(x);
        a.add_read(x);

        assert_eq!(a.self_conflict(), None);
        assert_eq!(a.reads(), &[x]);
    }

    #[test]
    fn listas_ficam_ordenadas_independentemente_da_ordem_de_insercao() {
        let (x, y, z) = ids();
        let mut a = Access::new();
        a.add_read(z);
        a.add_read(x);
        a.add_read(y);

        assert_eq!(a.reads(), &[x, y, z]);
    }
}
