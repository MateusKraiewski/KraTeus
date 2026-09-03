//! Iteracao tipada sobre entidades que casam com um conjunto de componentes.
//!
//! ```
//! use krateus_ecs::{With, World};
//!
//! struct Posicao(f32);
//! struct Velocidade(f32);
//! struct Viva;
//!
//! let mut world = World::new();
//! world.spawn((Posicao(0.0), Velocidade(2.0), Viva));
//! world.spawn((Posicao(0.0), Velocidade(5.0)));
//!
//! // So as entidades vivas avancam.
//! for (p, v) in world.query_filtered::<(&mut Posicao, &Velocidade), With<Viva>>() {
//!     p.0 += v.0;
//! }
//! ```
//!
//! # Como o casamento acontece
//!
//! A query nao percorre entidades para decidir quais servem: ela percorre
//! archetypes. Um archetype inteiro casa ou nao casa, e a decisao sai da
//! assinatura dele. Com isso o custo de selecao e proporcional ao numero de
//! archetypes — pequeno e estavel — e nao ao numero de entidades.
//!
//! # Por que isto e seguro
//!
//! A iteracao entrega `&mut T` a partir de um `&Archetypes`. O argumento que
//! sustenta isso tem tres partes:
//!
//! 1. Uma query so nasce de `&mut World`, e segura esse emprestimo por toda a
//!    sua vida. Enquanto ela existe, mais nada acessa o mundo.
//! 2. Os ponteiros vem de `Column::base`, que devolve a proveniencia da
//!    alocacao, e nao a da referencia compartilhada usada para le-la.
//! 3. Componentes distintos moram em colunas distintas, e
//!    [`Access::self_conflict`] rejeita, na construcao, qualquer query que peca
//!    o mesmo componente de forma incompativel. Duas linhas diferentes da mesma
//!    coluna tambem nao se sobrepoem, e a iteracao nunca visita a mesma linha
//!    duas vezes.

use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::access::{Access, Conflict};
use crate::archetype::{Archetype, ArchetypeId, Archetypes};
use crate::component::{Component, ComponentId, Components};
use crate::entity::Entity;

/// Ponteiro para o inicio de uma coluna, com o tempo de vida do archetype.
pub struct ColumnBase<'w, T> {
    base: NonNull<T>,
    _marker: PhantomData<&'w [T]>,
}

// Manuais porque `derive` exigiria `T: Copy`, e o que se copia aqui e o
// ponteiro, nunca o componente.
impl<T> Clone for ColumnBase<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for ColumnBase<'_, T> {}

/// O que uma query le de cada entidade.
///
/// Implementado para [`Entity`], `&T`, `&mut T` e tuplas de ate 8 desses.
///
/// O tempo de vida do mundo aparece como parametro dos tipos associados
/// [`Item`](QueryData::Item) e [`Fetch`](QueryData::Fetch), e nao do proprio
/// trait. Isso deixa [`State`](QueryData::State) livre de tempo de vida, que e
/// o que permite a um sistema guardar o estado da sua query entre execucoes.
///
/// Nas implementacoes, as assinaturas de `init_fetch` e `fetch` precisam usar a
/// **forma projetada** — `Self::Fetch<'w>`, `Self::Item<'w>` — e nunca o tipo
/// concreto equivalente. Escrever o tipo concreto normaliza a projecao, muda o
/// `'w` de early-bound para late-bound e produz `E0195` contra a declaracao do
/// trait.
///
/// # Safety
///
/// Quem implementa precisa garantir que [`matches`](QueryData::matches) so
/// aceite archetypes onde [`init_fetch`](QueryData::init_fetch) consiga obter
/// todas as colunas, e que [`access`](QueryData::access) declare exatamente os
/// componentes tocados — a checagem de autoconflito e o scheduler confiam nessa
/// declaracao para evitar aliasing.
pub unsafe trait QueryData {
    /// Valor entregue por entidade.
    type Item<'w>;
    /// Dados resolvidos uma vez por query. Sem tempo de vida, para poder ser
    /// guardado por um sistema.
    type State: Send + Sync + 'static;
    /// Ponteiros resolvidos uma vez por archetype.
    ///
    /// `Copy` porque e passado por valor a cada linha: sao ponteiros, e
    /// copia-los custa menos do que emprestar.
    type Fetch<'w>: Copy;

    /// Registra os componentes necessarios e devolve o estado da query.
    fn init_state(components: &mut Components) -> Self::State;

    /// Declara o que sera lido e escrito.
    fn access(state: &Self::State, out: &mut Access);

    /// Indica se o archetype tem tudo o que a query pede.
    fn matches(state: &Self::State, archetype: &Archetype) -> bool;

    /// Resolve os ponteiros das colunas deste archetype.
    ///
    /// # Safety
    ///
    /// `archetype` precisa ter sido aceito por [`matches`](QueryData::matches).
    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w>;

    /// Le a linha `row`.
    ///
    /// # Safety
    ///
    /// `row` precisa ser menor que o numero de entidades do archetype passado a
    /// [`init_fetch`](QueryData::init_fetch), e a mesma linha nao pode ser lida
    /// duas vezes enquanto o item anterior existir, no caso de acesso mutavel.
    unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w>;
}

// SAFETY: `Entity` nao acessa componente nenhum, entao casa com qualquer
// archetype e nao declara acesso. As entidades vem da fatia do proprio
// archetype, cujo comprimento e o mesmo usado para limitar `row`.
unsafe impl QueryData for Entity {
    type Item<'w> = Entity;
    type State = ();
    type Fetch<'w> = &'w [Entity];

    fn init_state(_: &mut Components) -> Self::State {}

    fn access(_: &Self::State, _: &mut Access) {}

    fn matches(_: &Self::State, _: &Archetype) -> bool {
        true
    }

    unsafe fn init_fetch<'w>(_: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w> {
        archetype.entities()
    }

    unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        debug_assert!(row < fetch.len());
        // SAFETY: o contrato exige `row` dentro do archetype, e esta fatia tem
        // exatamente uma entrada por entidade dele.
        unsafe { *fetch.get_unchecked(row) }
    }
}

// SAFETY: `matches` exige a coluna de `T`, que `init_fetch` entao obtem sem
// falhar. `access` declara leitura de `T`, e o item devolvido e compartilhado.
unsafe impl<T: Component> QueryData for &T {
    type Item<'w> = &'w T;
    type State = ComponentId;
    type Fetch<'w> = ColumnBase<'w, T>;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn access(state: &ComponentId, out: &mut Access) {
        out.add_component_read(*state);
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w> {
        let coluna = archetype.column(*state).expect("matches aceitou este archetype");
        ColumnBase { base: coluna.base().cast::<T>(), _marker: PhantomData }
    }

    unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY: `row` esta dentro da coluna pelo contrato, e o invariante 2
        // do `World` garante que essa posicao esta inicializada. A proveniencia
        // do ponteiro e a da alocacao da coluna.
        unsafe { &*fetch.base.as_ptr().add(row) }
    }
}

// SAFETY: como em `&T`, mas declarando escrita. A exclusividade da referencia
// devolvida vem de tres fatos: a query nasce de `&mut World`; `self_conflict`
// rejeita pedir o mesmo componente duas vezes; e a iteracao visita cada linha
// uma unica vez.
unsafe impl<T: Component> QueryData for &mut T {
    type Item<'w> = &'w mut T;
    type State = ComponentId;
    type Fetch<'w> = ColumnBase<'w, T>;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn access(state: &ComponentId, out: &mut Access) {
        out.add_component_write(*state);
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w> {
        let coluna = archetype.column(*state).expect("matches aceitou este archetype");
        ColumnBase { base: coluna.base().cast::<T>(), _marker: PhantomData }
    }

    unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY: ver o comentario da implementacao. Nenhuma outra referencia
        // viva alcanca esta linha desta coluna.
        unsafe { &mut *fetch.base.as_ptr().add(row) }
    }
}

macro_rules! impl_query_data_tupla {
    ($(($D:ident, $i:tt)),+) => {
        // SAFETY: cada elemento cumpre o proprio contrato; a tupla apenas
        // encaminha `matches`, `access` e `fetch` para todos eles. `matches` so
        // aceita quando todos aceitam, entao todo `init_fetch` tem suas colunas.
        unsafe impl<$($D: QueryData),+> QueryData for ($($D,)+) {
            type Item<'w> = ($($D::Item<'w>,)+);
            type State = ($($D::State,)+);
            type Fetch<'w> = ($($D::Fetch<'w>,)+);

            fn init_state(components: &mut Components) -> Self::State {
                ($($D::init_state(components),)+)
            }

            fn access(state: &Self::State, out: &mut Access) {
                $($D::access(&state.$i, out);)+
            }

            fn matches(state: &Self::State, archetype: &Archetype) -> bool {
                $($D::matches(&state.$i, archetype))&&+
            }

            unsafe fn init_fetch<'w>(
                state: &Self::State,
                archetype: &'w Archetype,
            ) -> Self::Fetch<'w> {
                // SAFETY: `matches` aceitou, logo aceitou para cada elemento.
                unsafe { ($($D::init_fetch(&state.$i, archetype),)+) }
            }

            unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
                // SAFETY: `row` valido pelo contrato; a ausencia de conflito
                // entre os elementos foi verificada na criacao da query.
                unsafe { ($($D::fetch(fetch.$i, row),)+) }
            }
        }
    };
}

impl_query_data_tupla!((A, 0), (B, 1));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2), (D, 3));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));
impl_query_data_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6), (H, 7));

/// Condicao que um archetype precisa satisfazer, sem que nada seja lido dele.
pub trait QueryFilter {
    /// Dados resolvidos uma vez por query.
    type State: Send + Sync;

    /// Registra os componentes envolvidos e devolve o estado do filtro.
    fn init_state(components: &mut Components) -> Self::State;

    /// Indica se o archetype passa.
    fn matches(state: &Self::State, archetype: &Archetype) -> bool;
}

/// Filtro vazio: aceita todo archetype.
impl QueryFilter for () {
    type State = ();

    fn init_state(_: &mut Components) -> Self::State {}

    fn matches(_: &Self::State, _: &Archetype) -> bool {
        true
    }
}

/// Exige a presenca de `T` sem le-lo.
///
/// Serve a componentes-marcador, que nao carregam dado: `With<Viva>` filtra sem
/// obrigar a query a devolver um `&Viva` que ninguem usaria.
pub struct With<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for With<T> {
    type State = ComponentId;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }
}

/// Exige a ausencia de `T`.
pub struct Without<T>(PhantomData<fn() -> T>);

impl<T: Component> QueryFilter for Without<T> {
    type State = ComponentId;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        !archetype.contains(*state)
    }
}

macro_rules! impl_query_filter_tupla {
    ($(($F:ident, $i:tt)),+) => {
        impl<$($F: QueryFilter),+> QueryFilter for ($($F,)+) {
            type State = ($($F::State,)+);

            fn init_state(components: &mut Components) -> Self::State {
                ($($F::init_state(components),)+)
            }

            fn matches(state: &Self::State, archetype: &Archetype) -> bool {
                $($F::matches(&state.$i, archetype))&&+
            }
        }
    };
}

impl_query_filter_tupla!((A, 0));
impl_query_filter_tupla!((A, 0), (B, 1));
impl_query_filter_tupla!((A, 0), (B, 1), (C, 2));
impl_query_filter_tupla!((A, 0), (B, 1), (C, 2), (D, 3));

/// Percorre as entidades que casam com `D` e passam por `F`.
///
/// A ordem e: archetypes na ordem de criacao, e dentro de cada um, linhas em
/// ordem crescente. Estavel entre execucoes, como pede a secao 15 do documento
/// de visao.
pub struct QueryIter<'w, D: QueryData, F: QueryFilter = ()> {
    archetypes: &'w Archetypes,
    state: D::State,
    correspondentes: Vec<ArchetypeId>,
    proximo: usize,
    fetch: Option<D::Fetch<'w>>,
    row: usize,
    len: usize,
    restantes: usize,
    _filtro: PhantomData<fn() -> F>,
}

impl<'w, D: QueryData, F: QueryFilter> QueryIter<'w, D, F> {
    /// Monta a query, resolvendo os archetypes que casam.
    ///
    /// # Panics
    ///
    /// Se `D` pedir o mesmo componente de forma conflitante.
    pub(crate) fn new(archetypes: &'w Archetypes, components: &mut Components) -> Self {
        let state = D::init_state(components);
        let filtro = F::init_state(components);

        let mut acesso = Access::new();
        D::access(&state, &mut acesso);
        if let Some(Conflict::Component(id)) = acesso.self_conflict() {
            let nome = components.info(id).map_or("?", |i| i.name());
            panic!("query pede o componente {nome} de forma conflitante consigo mesma");
        }

        let mut correspondentes = Vec::new();
        let mut restantes = 0;
        for archetype in archetypes.iter() {
            if D::matches(&state, archetype) && F::matches(&filtro, archetype) {
                restantes += archetype.len();
                correspondentes.push(archetype.id());
            }
        }

        Self {
            archetypes,
            state,
            correspondentes,
            proximo: 0,
            fetch: None,
            row: 0,
            len: 0,
            restantes,
            _filtro: PhantomData,
        }
    }
}

impl<'w, D: QueryData, F: QueryFilter> Iterator for QueryIter<'w, D, F> {
    type Item = D::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.row < self.len {
                let fetch = self.fetch.expect("ha linhas, logo ha fetch");
                // SAFETY: `row < len`, e `len` e o numero de entidades do
                // archetype de onde `fetch` foi construido. Cada linha e
                // visitada uma unica vez, ja que `row` so cresce.
                let item = unsafe { D::fetch(fetch, self.row) };
                self.row += 1;
                self.restantes -= 1;
                return Some(item);
            }

            let id = *self.correspondentes.get(self.proximo)?;
            self.proximo += 1;

            let archetypes: &'w Archetypes = self.archetypes;
            let archetype = archetypes.get(id).expect("id veio desta colecao");
            // SAFETY: `id` so entrou em `correspondentes` depois de `matches`
            // aceitar este archetype.
            self.fetch = Some(unsafe { D::init_fetch(&self.state, archetype) });
            self.row = 0;
            self.len = archetype.len();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.restantes, Some(self.restantes))
    }
}

impl<D: QueryData, F: QueryFilter> ExactSizeIterator for QueryIter<'_, D, F> {}

#[cfg(test)]
mod tests {
    use crate::{Entity, With, Without, World};

    #[derive(Debug, PartialEq)]
    struct Posicao(f32);
    #[derive(Debug, PartialEq)]
    struct Velocidade(f32);
    struct Viva;
    struct Congelada;

    #[test]
    fn itera_apenas_quem_tem_todos_os_componentes() {
        let mut w = World::new();
        w.spawn((Posicao(1.0), Velocidade(10.0)));
        w.spawn((Posicao(2.0),));
        w.spawn((Velocidade(20.0),));

        let vistos: Vec<_> =
            w.query::<(&Posicao, &Velocidade)>().map(|(p, v)| (p.0, v.0)).collect();

        assert_eq!(vistos, vec![(1.0, 10.0)]);
    }

    #[test]
    fn escrita_pela_query_persiste() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0), Velocidade(3.0)));

        for (p, v) in w.query::<(&mut Posicao, &Velocidade)>() {
            p.0 += v.0;
        }

        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(3.0)));
    }

    #[test]
    fn query_atravessa_archetypes_distintos() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0), Velocidade(0.0)));
        w.spawn((Posicao(3.0), Viva));

        let mut vistos: Vec<_> = w.query::<&Posicao>().map(|p| p.0).collect();
        vistos.sort_by(f32::total_cmp);

        assert_eq!(vistos, vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn entity_pode_ser_pedida_junto_com_componentes() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        let b = w.spawn((Posicao(2.0),));

        let vistos: Vec<_> = w.query::<(Entity, &Posicao)>().map(|(e, p)| (e, p.0)).collect();

        assert_eq!(vistos, vec![(a, 1.0), (b, 2.0)]);
    }

    #[test]
    fn with_filtra_sem_devolver_o_componente() {
        let mut w = World::new();
        w.spawn((Posicao(1.0), Viva));
        w.spawn((Posicao(2.0),));

        let vistos: Vec<_> = w.query_filtered::<&Posicao, With<Viva>>().map(|p| p.0).collect();

        assert_eq!(vistos, vec![1.0]);
    }

    #[test]
    fn without_exclui_quem_tem_o_componente() {
        let mut w = World::new();
        w.spawn((Posicao(1.0), Congelada));
        w.spawn((Posicao(2.0),));

        let vistos: Vec<_> =
            w.query_filtered::<&Posicao, Without<Congelada>>().map(|p| p.0).collect();

        assert_eq!(vistos, vec![2.0]);
    }

    #[test]
    fn filtros_podem_ser_combinados() {
        let mut w = World::new();
        w.spawn((Posicao(1.0), Viva));
        w.spawn((Posicao(2.0), Viva, Congelada));
        w.spawn((Posicao(3.0),));

        let vistos: Vec<_> =
            w.query_filtered::<&Posicao, (With<Viva>, Without<Congelada>)>().map(|p| p.0).collect();

        assert_eq!(vistos, vec![1.0]);
    }

    #[test]
    fn query_sem_correspondencia_nao_itera() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));

        assert_eq!(w.query::<&Velocidade>().count(), 0);
    }

    #[test]
    fn size_hint_bate_com_a_contagem_real() {
        let mut w = World::new();
        for i in 0..7 {
            w.spawn((Posicao(i as f32),));
        }
        w.spawn((Velocidade(0.0),));

        assert_eq!(w.query::<&Posicao>().len(), 7);
        assert_eq!(w.query::<&Posicao>().count(), 7);
    }

    #[test]
    #[should_panic(expected = "conflitante")]
    fn query_com_dois_acessos_mutaveis_ao_mesmo_componente_falha_alto() {
        let mut w = World::new();
        w.spawn((Posicao(0.0),));
        let _ = w.query::<(&mut Posicao, &mut Posicao)>().count();
    }

    #[test]
    #[should_panic(expected = "conflitante")]
    fn query_que_le_e_escreve_o_mesmo_componente_falha_alto() {
        let mut w = World::new();
        w.spawn((Posicao(0.0),));
        let _ = w.query::<(&Posicao, &mut Posicao)>().count();
    }

    #[test]
    fn ordem_de_iteracao_e_deterministica() {
        let executar = || {
            let mut w = World::new();
            for i in 0..30 {
                if i % 3 == 0 {
                    w.spawn((Posicao(i as f32), Velocidade(0.0)));
                } else {
                    w.spawn((Posicao(i as f32),));
                }
            }
            w.query::<&Posicao>().map(|p| p.0.to_bits()).collect::<Vec<_>>()
        };

        assert_eq!(executar(), executar());
    }

    #[test]
    fn muitas_entidades_sao_todas_visitadas_uma_vez() {
        let mut w = World::new();
        for i in 0..10_000 {
            w.spawn((Posicao(i as f32), Velocidade(1.0)));
        }

        let mut visitas = 0;
        for (p, v) in w.query::<(&mut Posicao, &Velocidade)>() {
            p.0 += v.0;
            visitas += 1;
        }
        assert_eq!(visitas, 10_000);

        // Cada posicao virou i + 1, entao a soma e a de 1..=10_000.
        let soma: f64 = w.query::<&Posicao>().map(|p| f64::from(p.0)).sum();
        assert_eq!(soma as u64, (1..=10_000_u64).sum::<u64>());
    }

    #[test]
    fn componente_marcador_zst_funciona_como_dado() {
        let mut w = World::new();
        w.spawn((Posicao(1.0), Viva));
        w.spawn((Posicao(2.0), Viva));

        assert_eq!(w.query::<(&Posicao, &Viva)>().count(), 2);
    }

    #[test]
    fn entidades_deslocadas_por_despawn_continuam_visiveis() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0),));
        w.spawn((Posicao(3.0),));

        w.despawn(a);

        let mut vistos: Vec<_> = w.query::<&Posicao>().map(|p| p.0).collect();
        vistos.sort_by(f32::total_cmp);
        assert_eq!(vistos, vec![2.0, 3.0]);
    }
}
