//! Identidade de entidade e tabela de localizacao.
//!
//! Uma entidade nao guarda dado nenhum: e um par `(indice, geracao)` que serve
//! de chave para descobrir em qual archetype e em qual linha os componentes
//! dela estao. Essa indirecao e o que permite mover uma entidade entre
//! archetypes quando um componente e adicionado ou removido, sem invalidar a
//! referencia que o codigo de jogo guardou.
//!
//! A geracao resolve o mesmo problema que resolve no `Pool` do core: sem ela,
//! reciclar um indice faria uma referencia antiga passar a apontar, em
//! silencio, para outra entidade.

use std::fmt;
use std::num::NonZeroU32;

use crate::archetype::ArchetypeId;

const PRIMEIRA_GERACAO: NonZeroU32 = NonZeroU32::new(1).unwrap();

/// Identificador de uma entidade no [`World`](crate::World).
///
/// Ocupa 8 bytes e e `Copy`. Comparacao e ordenacao consideram indice e
/// geracao, nessa ordem — a ordem total existe para permitir iteracao
/// deterministica quando o codigo precisar ordenar entidades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Entity {
    index: u32,
    generation: NonZeroU32,
}

impl Entity {
    /// Constroi uma entidade a partir de suas partes.
    ///
    /// Exposto para desserializacao e testes. Em uso normal, entidades vem de
    /// [`World::spawn`](crate::World::spawn).
    #[inline]
    #[must_use]
    pub const fn from_raw(index: u32, generation: NonZeroU32) -> Self {
        Self { index, generation }
    }

    /// Indice do slot ocupado por esta entidade.
    #[inline]
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Geracao do slot no momento em que esta entidade foi criada.
    #[inline]
    #[must_use]
    pub const fn generation(self) -> NonZeroU32 {
        self.generation
    }
}

impl fmt::Display for Entity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}v{}", self.index, self.generation)
    }
}

/// Onde os componentes de uma entidade viva estao guardados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityLocation {
    /// Archetype que contem a entidade.
    pub archetype: ArchetypeId,
    /// Linha dentro das colunas daquele archetype.
    pub row: u32,
}

#[derive(Debug)]
struct EntityMeta {
    generation: NonZeroU32,
    location: Option<EntityLocation>,
}

/// Alocador de entidades e tabela de localizacao.
///
/// Mantem, para cada indice ja usado, a geracao corrente e onde a entidade
/// esta. Indices liberados voltam para uma pilha e sao reutilizados em ordem
/// LIFO — deterministica, portanto reproduzivel entre execucoes.
#[derive(Debug, Default)]
pub struct Entities {
    metas: Vec<EntityMeta>,
    livres: Vec<u32>,
    vivas: usize,
}

impl Entities {
    /// Cria uma tabela vazia, sem alocar.
    #[must_use]
    pub const fn new() -> Self {
        Self { metas: Vec::new(), livres: Vec::new(), vivas: 0 }
    }

    /// Quantidade de entidades vivas.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.vivas
    }

    /// Indica se nao ha entidades vivas.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.vivas == 0
    }

    /// Reserva espaco para `n` entidades adicionais.
    pub fn reserve(&mut self, n: usize) {
        self.metas.reserve(n);
    }

    /// Aloca uma entidade nova, ainda sem localizacao.
    ///
    /// Quem chama e responsavel por inseri-la em um archetype e registrar a
    /// posicao com [`set_location`](Self::set_location).
    ///
    /// # Panics
    ///
    /// Se o numero de slots exceder `u32::MAX`.
    pub fn alloc(&mut self) -> Entity {
        self.vivas += 1;

        if let Some(index) = self.livres.pop() {
            let meta = &mut self.metas[index as usize];
            meta.location = None;
            return Entity { index, generation: meta.generation };
        }

        let index = u32::try_from(self.metas.len()).expect("mais de u32::MAX entidades");
        self.metas.push(EntityMeta { generation: PRIMEIRA_GERACAO, location: None });
        Entity { index, generation: PRIMEIRA_GERACAO }
    }

    /// Indica se a entidade ainda existe.
    #[must_use]
    pub fn contains(&self, entity: Entity) -> bool {
        self.meta(entity).is_some()
    }

    /// Onde a entidade esta, ou `None` se ela nao existe ou ainda nao foi
    /// inserida em um archetype.
    #[must_use]
    pub fn location(&self, entity: Entity) -> Option<EntityLocation> {
        self.meta(entity)?.location
    }

    /// Registra a posicao da entidade.
    ///
    /// Devolve `false` se a entidade nao existe mais.
    pub fn set_location(&mut self, entity: Entity, location: EntityLocation) -> bool {
        let Some(meta) = self.meta_mut(entity) else {
            return false;
        };
        meta.location = Some(location);
        true
    }

    /// Libera a entidade e devolve onde ela estava, se estava em algum lugar.
    ///
    /// O slot volta para a pilha de livres com a geracao incrementada. Quando a
    /// geracao satura, o slot e aposentado em vez de reciclado: vazar um slot e
    /// mais barato que arriscar duas entidades com a mesma identidade.
    pub fn free(&mut self, entity: Entity) -> Option<EntityLocation> {
        let index = entity.index() as usize;
        let meta = self.metas.get_mut(index)?;
        if meta.generation != entity.generation() {
            return None;
        }

        let location = meta.location.take();
        self.vivas -= 1;

        match meta.generation.checked_add(1) {
            Some(proxima) => {
                meta.generation = proxima;
                self.livres.push(entity.index());
            }
            None => tracing::warn!(slot = index, "geracao de entidade saturada; slot aposentado"),
        }

        location
    }

    /// Itera sobre as entidades vivas em ordem de indice.
    ///
    /// A ordem e estavel entre execucoes — requisito dos testes de determinismo
    /// da secao 15 do documento de visao.
    pub fn iter(&self) -> impl Iterator<Item = Entity> {
        self.metas.iter().enumerate().filter_map(|(i, meta)| {
            meta.location.map(|_| Entity { index: i as u32, generation: meta.generation })
        })
    }

    fn meta(&self, entity: Entity) -> Option<&EntityMeta> {
        let meta = self.metas.get(entity.index() as usize)?;
        (meta.generation == entity.generation()).then_some(meta)
    }

    fn meta_mut(&mut self, entity: Entity) -> Option<&mut EntityMeta> {
        let meta = self.metas.get_mut(entity.index() as usize)?;
        (meta.generation == entity.generation()).then_some(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(row: u32) -> EntityLocation {
        EntityLocation { archetype: ArchetypeId::VAZIO, row }
    }

    #[test]
    fn aloca_indices_crescentes() {
        let mut e = Entities::new();
        assert_eq!(e.alloc().index(), 0);
        assert_eq!(e.alloc().index(), 1);
        assert_eq!(e.len(), 2);
    }

    #[test]
    fn entidade_liberada_deixa_de_existir() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.set_location(a, loc(0));

        assert_eq!(e.free(a), Some(loc(0)));
        assert!(!e.contains(a));
        assert!(e.is_empty());
    }

    #[test]
    fn slot_reciclado_nao_colide_com_entidade_antiga() {
        let mut e = Entities::new();
        let antiga = e.alloc();
        e.free(antiga);
        let nova = e.alloc();

        assert_eq!(antiga.index(), nova.index());
        assert_ne!(antiga, nova);
        assert!(!e.contains(antiga));
        assert!(e.contains(nova));
    }

    #[test]
    fn liberar_duas_vezes_nao_corrompe_a_contagem() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.set_location(a, loc(0));

        assert_eq!(e.free(a), Some(loc(0)));
        assert!(e.is_empty());

        // A segunda liberacao encontra geracao diferente e sai sem mexer na
        // contagem. Sem essa checagem, `vivas` faria underflow.
        assert_eq!(e.free(a), None);
        assert!(e.is_empty());
    }

    #[test]
    fn reciclagem_e_lifo_e_portanto_deterministica() {
        let executar = || {
            let mut e = Entities::new();
            let entidades: Vec<_> = (0..8).map(|_| e.alloc()).collect();
            for ent in entidades.iter().take(5) {
                e.free(*ent);
            }
            (0..5).map(|_| e.alloc().index()).collect::<Vec<_>>()
        };

        assert_eq!(executar(), vec![4, 3, 2, 1, 0]);
        assert_eq!(executar(), executar());
    }

    #[test]
    fn iter_devolve_apenas_entidades_posicionadas() {
        let mut e = Entities::new();
        let a = e.alloc();
        let b = e.alloc();
        let _sem_lugar = e.alloc();

        e.set_location(a, loc(0));
        e.set_location(b, loc(1));

        assert_eq!(e.iter().collect::<Vec<_>>(), vec![a, b]);
    }

    #[test]
    fn set_location_em_entidade_morta_falha() {
        let mut e = Entities::new();
        let a = e.alloc();
        e.free(a);
        assert!(!e.set_location(a, loc(0)));
    }
}
