//! Conjunto de componentes e recursos lidos e escritos por uma query ou sistema.
//!
//! Serve a dois donos, e o segundo e a razao de este tipo existir tao cedo:
//!
//! 1. Rejeitar, na criacao, uma query que conflitaria consigo mesma — como
//!    `(&mut Posicao, &mut Posicao)`, que entregaria duas referencias mutaveis
//!    ao mesmo valor. E a diferenca entre um `panic` claro e comportamento
//!    indefinido.
//! 2. Alimentar o scheduler da Fase 2. Sistemas cujos acessos nao colidem podem
//!    rodar em paralelo, e essa decisao sai daqui — derivada dos tipos, sem que
//!    ninguem precise ordenar sistemas a mao (secao 5 do documento de visao).
//!
//! Componentes e recursos vivem em **espacos separados**. Um componente
//! `Posicao` e um recurso `Posicao` sao coisas distintas: nada justifica que um
//! sistema que escreve o componente espere pelo que le o recurso.

use crate::component::ComponentId;
use crate::resource::ResourceId;

/// O que impede dois acessos de coexistirem.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    /// Componente pedido de forma incompativel.
    Component(ComponentId),
    /// Recurso pedido de forma incompativel.
    Resource(ResourceId),
}

/// Leituras e escritas sobre um unico espaco de identificadores.
///
/// As listas ficam ordenadas e sem repeticao, o que torna as comparacoes uma
/// intersecao linear em vez de um produto cartesiano.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Espaco<T> {
    leituras: Vec<T>,
    escritas: Vec<T>,
    /// Registra que algum identificador foi pedido para escrita mais de uma vez.
    /// Guardado a parte porque `escritas` e desduplicada.
    escrita_duplicada: Option<T>,
}

// Manual: `derive` exigiria `T: Default`, que nao faz sentido para um id.
impl<T> Default for Espaco<T> {
    fn default() -> Self {
        Self { leituras: Vec::new(), escritas: Vec::new(), escrita_duplicada: None }
    }
}

impl<T: Copy + Ord> Espaco<T> {
    fn add_read(&mut self, id: T) {
        insere_ordenado(&mut self.leituras, id);
    }

    fn add_write(&mut self, id: T) {
        if !insere_ordenado(&mut self.escritas, id) {
            self.escrita_duplicada.get_or_insert(id);
        }
    }

    fn is_empty(&self) -> bool {
        self.leituras.is_empty() && self.escritas.is_empty()
    }

    fn self_conflict(&self) -> Option<T> {
        self.escrita_duplicada.or_else(|| primeira_intersecao(&self.leituras, &self.escritas))
    }

    fn conflict_with(&self, outro: &Self) -> Option<T> {
        primeira_intersecao(&self.escritas, &outro.escritas)
            .or_else(|| primeira_intersecao(&self.escritas, &outro.leituras))
            .or_else(|| primeira_intersecao(&self.leituras, &outro.escritas))
    }
}

/// Componentes e recursos que uma query ou sistema toca.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Access {
    componentes: Espaco<ComponentId>,
    recursos: Espaco<ResourceId>,
}

impl Access {
    /// Cria um acesso vazio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra leitura de um componente.
    pub fn add_component_read(&mut self, id: ComponentId) {
        self.componentes.add_read(id);
    }

    /// Registra escrita de um componente.
    pub fn add_component_write(&mut self, id: ComponentId) {
        self.componentes.add_write(id);
    }

    /// Registra leitura de um recurso.
    pub fn add_resource_read(&mut self, id: ResourceId) {
        self.recursos.add_read(id);
    }

    /// Registra escrita de um recurso.
    pub fn add_resource_write(&mut self, id: ResourceId) {
        self.recursos.add_write(id);
    }

    /// Componentes lidos, ordenados.
    #[inline]
    #[must_use]
    pub fn component_reads(&self) -> &[ComponentId] {
        &self.componentes.leituras
    }

    /// Componentes escritos, ordenados.
    #[inline]
    #[must_use]
    pub fn component_writes(&self) -> &[ComponentId] {
        &self.componentes.escritas
    }

    /// Recursos lidos, ordenados.
    #[inline]
    #[must_use]
    pub fn resource_reads(&self) -> &[ResourceId] {
        &self.recursos.leituras
    }

    /// Recursos escritos, ordenados.
    #[inline]
    #[must_use]
    pub fn resource_writes(&self) -> &[ResourceId] {
        &self.recursos.escritas
    }

    /// Incorpora tudo o que `outro` declara.
    ///
    /// Usado para somar os acessos dos parametros de um sistema num acesso
    /// unico, que e o que o scheduler compara.
    pub fn extend(&mut self, outro: &Self) {
        for &id in &outro.componentes.leituras {
            self.add_component_read(id);
        }
        for &id in &outro.componentes.escritas {
            self.add_component_write(id);
        }
        for &id in &outro.recursos.leituras {
            self.add_resource_read(id);
        }
        for &id in &outro.recursos.escritas {
            self.add_resource_write(id);
        }
    }

    /// Indica se o acesso nao toca em nada.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.componentes.is_empty() && self.recursos.is_empty()
    }

    /// O que este acesso pede de forma incompativel consigo mesmo.
    ///
    /// Acontece quando o mesmo identificador e escrito duas vezes, ou lido e
    /// escrito ao mesmo tempo.
    #[must_use]
    pub fn self_conflict(&self) -> Option<Conflict> {
        self.componentes
            .self_conflict()
            .map(Conflict::Component)
            .or_else(|| self.recursos.self_conflict().map(Conflict::Resource))
    }

    /// O que impede este acesso de rodar em paralelo com `outro`.
    ///
    /// Duas leituras nunca colidem. Qualquer escrita colide com leitura ou
    /// escrita do mesmo identificador, no mesmo espaco.
    #[must_use]
    pub fn conflict_with(&self, outro: &Self) -> Option<Conflict> {
        self.componentes
            .conflict_with(&outro.componentes)
            .map(Conflict::Component)
            .or_else(|| self.recursos.conflict_with(&outro.recursos).map(Conflict::Resource))
    }

    /// Indica se os dois acessos podem rodar ao mesmo tempo.
    #[inline]
    #[must_use]
    pub fn compatible_with(&self, outro: &Self) -> bool {
        self.conflict_with(outro).is_none()
    }
}

/// Insere mantendo a ordem. Devolve `false` se ja estava presente.
fn insere_ordenado<T: Ord>(v: &mut Vec<T>, id: T) -> bool {
    match v.binary_search(&id) {
        Ok(_) => false,
        Err(pos) => {
            v.insert(pos, id);
            true
        }
    }
}

/// Primeiro elemento comum a duas listas ordenadas.
fn primeira_intersecao<T: Copy + Ord>(a: &[T], b: &[T]) -> Option<T> {
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
    use crate::resource::Resources;

    fn componentes() -> (ComponentId, ComponentId) {
        let mut c = Components::new();
        (c.register::<u8>(), c.register::<u16>())
    }

    fn recursos() -> (ResourceId, ResourceId) {
        let mut r = Resources::new();
        (r.register::<u8>(), r.register::<u16>())
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
        let (x, y) = componentes();
        let mut a = Access::new();
        a.add_component_read(x);
        a.add_component_read(y);

        let mut b = Access::new();
        b.add_component_read(x);

        assert!(a.compatible_with(&b));
    }

    #[test]
    fn escrita_colide_com_leitura_do_mesmo_componente() {
        let (x, _) = componentes();
        let mut escritor = Access::new();
        escritor.add_component_write(x);

        let mut leitor = Access::new();
        leitor.add_component_read(x);

        assert_eq!(escritor.conflict_with(&leitor), Some(Conflict::Component(x)));
        assert_eq!(leitor.conflict_with(&escritor), Some(Conflict::Component(x)));
    }

    #[test]
    fn escritas_de_componentes_distintos_sao_paralelizaveis() {
        let (x, y) = componentes();
        let mut a = Access::new();
        a.add_component_write(x);
        let mut b = Access::new();
        b.add_component_write(y);

        assert!(a.compatible_with(&b));
    }

    #[test]
    fn escrever_duas_vezes_o_mesmo_componente_e_autoconflito() {
        let (x, _) = componentes();
        let mut a = Access::new();
        a.add_component_write(x);
        a.add_component_write(x);

        assert_eq!(a.self_conflict(), Some(Conflict::Component(x)));
    }

    #[test]
    fn ler_e_escrever_o_mesmo_componente_e_autoconflito() {
        let (x, _) = componentes();
        let mut a = Access::new();
        a.add_component_read(x);
        a.add_component_write(x);

        assert_eq!(a.self_conflict(), Some(Conflict::Component(x)));
    }

    #[test]
    fn ler_o_mesmo_componente_duas_vezes_e_permitido() {
        let (x, _) = componentes();
        let mut a = Access::new();
        a.add_component_read(x);
        a.add_component_read(x);

        assert_eq!(a.self_conflict(), None);
        assert_eq!(a.component_reads(), &[x]);
    }

    #[test]
    fn recursos_conflitam_entre_si_como_componentes() {
        let (x, _) = recursos();
        let mut escritor = Access::new();
        escritor.add_resource_write(x);

        let mut leitor = Access::new();
        leitor.add_resource_read(x);

        assert_eq!(escritor.conflict_with(&leitor), Some(Conflict::Resource(x)));
        assert!(!escritor.compatible_with(&leitor));
    }

    #[test]
    fn componente_e_recurso_de_mesmo_indice_nao_se_confundem() {
        let (comp, _) = componentes();
        let (rec, _) = recursos();
        // Os dois tem indice 0, em espacos diferentes.
        assert_eq!(comp.index(), rec.index());

        let mut escreve_componente = Access::new();
        escreve_componente.add_component_write(comp);

        let mut escreve_recurso = Access::new();
        escreve_recurso.add_resource_write(rec);

        assert!(
            escreve_componente.compatible_with(&escreve_recurso),
            "espacos distintos nao podem colidir"
        );
    }

    #[test]
    fn conflito_de_componente_tem_precedencia_na_mensagem() {
        let (c, _) = componentes();
        let (r, _) = recursos();

        let mut a = Access::new();
        a.add_component_write(c);
        a.add_resource_write(r);

        let mut b = Access::new();
        b.add_component_write(c);
        b.add_resource_write(r);

        // Ambos colidem; o relatorio devolve um, de forma estavel.
        assert_eq!(a.conflict_with(&b), Some(Conflict::Component(c)));
    }

    #[test]
    fn listas_ficam_ordenadas_independentemente_da_ordem_de_insercao() {
        // Os identificadores saem em ordem crescente de registro.
        let mut c = Components::new();
        let primeiro = c.register::<u8>();
        let segundo = c.register::<u16>();
        let terceiro = c.register::<u32>();

        let mut a = Access::new();
        a.add_component_read(terceiro);
        a.add_component_read(primeiro);
        a.add_component_read(segundo);

        assert_eq!(a.component_reads(), &[primeiro, segundo, terceiro]);
    }
}
