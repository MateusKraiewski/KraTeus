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
//!
//! # Deteccao de mudanca
//!
//! Pedir `&mut T` **carimba** a linha como alterada, tenha o valor mudado ou
//! nao: o contrato e "foi exposto para escrita" (ver D10 em
//! `docs/DECISOES.md`). [`Changed`] e [`Added`] leem esse carimbo.
//!
//! Ao contrario de [`With`] e [`Without`], que decidem por archetype, os
//! filtros de mudanca decidem **por linha**. O iterador so paga por isso quando
//! ha algum: [`QueryFilter::POR_LINHA`] e uma constante, e o desvio some da
//! compilacao quando ninguem precisa dele.

use std::cell::UnsafeCell;
use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::access::{Access, Conflict};
use crate::archetype::{Archetype, ArchetypeId, Archetypes};
use crate::component::{Component, ComponentId, Components};
use crate::entity::Entity;
use crate::tick::{ComponentTicks, SystemTicks, Tick};

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

/// Ponteiros de uma coluna aberta para escrita: os valores e os ticks.
///
/// Carrega o tick da execucao corrente porque entregar `&mut T` carimba a linha
/// como alterada — o contrato da D10 e "foi exposto para escrita".
pub struct WriteBase<'w, T> {
    valores: NonNull<T>,
    ticks: NonNull<UnsafeCell<ComponentTicks>>,
    atual: Tick,
    _marker: PhantomData<&'w [T]>,
}

impl<T> Clone for WriteBase<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for WriteBase<'_, T> {}

/// Ponteiro para uma coluna de ticks, para os filtros de mudanca.
#[derive(Clone, Copy)]
pub struct TicksBase<'w> {
    base: NonNull<UnsafeCell<ComponentTicks>>,
    _marker: PhantomData<&'w [ComponentTicks]>,
}

impl TicksBase<'_> {
    /// Le os ticks da linha `row`.
    ///
    /// # Safety
    ///
    /// `row` precisa estar dentro do archetype de onde este ponteiro veio, e
    /// nenhuma escrita concorrente pode estar ativa sobre esta coluna.
    #[inline]
    unsafe fn ler(self, row: usize) -> ComponentTicks {
        // SAFETY: o contrato garante `row` valido e ausencia de escrita
        // concorrente; a coluna existe enquanto `'w` durar.
        unsafe { *UnsafeCell::raw_get(self.base.as_ptr().add(row)) }
    }
}

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
    /// guardado por um sistema; `Clone` para o iterador levar uma copia sem
    /// prender o estado por emprestimo.
    type State: Send + Sync + Clone + 'static;
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
    unsafe fn init_fetch<'w>(
        state: &Self::State,
        archetype: &'w Archetype,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w>;

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

    unsafe fn init_fetch<'w>(
        _: &Self::State,
        archetype: &'w Archetype,
        _: SystemTicks,
    ) -> Self::Fetch<'w> {
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

    unsafe fn init_fetch<'w>(
        state: &Self::State,
        archetype: &'w Archetype,
        _: SystemTicks,
    ) -> Self::Fetch<'w> {
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
    type Fetch<'w> = WriteBase<'w, T>;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn access(state: &ComponentId, out: &mut Access) {
        out.add_component_write(*state);
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(
        state: &Self::State,
        archetype: &'w Archetype,
        ticks: SystemTicks,
    ) -> Self::Fetch<'w> {
        let i = archetype.column_index(*state).expect("matches aceitou este archetype");
        let coluna = archetype.column(*state).expect("matches aceitou este archetype");
        WriteBase {
            valores: coluna.base().cast::<T>(),
            ticks: archetype.ticks_base(i),
            atual: ticks.atual,
            _marker: PhantomData,
        }
    }

    unsafe fn fetch<'w>(fetch: Self::Fetch<'w>, row: usize) -> Self::Item<'w> {
        // SAFETY: `row` e valido pelo contrato, e a coluna de ticks tem uma
        // entrada por linha. O ponteiro vem de `UnsafeCell`, entao escrever por
        // ele a partir de `&Archetype` e legitimo; a exclusividade sobre esta
        // coluna e a mesma que autoriza o `&mut T` abaixo.
        unsafe {
            (*UnsafeCell::raw_get(fetch.ticks.as_ptr().add(row))).changed = fetch.atual;
        }

        // SAFETY: ver o comentario da implementacao. Nenhuma outra referencia
        // viva alcanca esta linha desta coluna.
        unsafe { &mut *fetch.valores.as_ptr().add(row) }
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
                ticks: SystemTicks,
            ) -> Self::Fetch<'w> {
                // SAFETY: `matches` aceitou, logo aceitou para cada elemento.
                unsafe { ($($D::init_fetch(&state.$i, archetype, ticks),)+) }
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

/// Condicao que uma entidade precisa satisfazer para entrar na iteracao.
///
/// Ha dois niveis. `With` e `Without` decidem **por archetype**: o conjunto
/// inteiro passa ou nao passa, e a iteracao nem chega a olhar linha por linha.
/// `Changed` e `Added` precisam decidir **por linha**, porque duas entidades do
/// mesmo archetype podem ter historias diferentes.
///
/// [`POR_LINHA`](QueryFilter::POR_LINHA) diz de qual tipo o filtro e. Quando
/// nenhum filtro exige avaliacao por linha, o iterador pula a checagem inteira —
/// e a diferenca entre o caso comum nao pagar nada e pagar um desvio por
/// entidade.
///
/// # Safety
///
/// Como em [`QueryData`], `access` precisa declarar o que `filter_fetch` le, e
/// `init_fetch` so pode ser chamado sobre archetype aceito por `matches`.
pub unsafe trait QueryFilter {
    /// Dados resolvidos uma vez por query.
    type State: Send + Sync + Clone + 'static;
    /// Ponteiros resolvidos uma vez por archetype.
    type Fetch<'w>: Copy;

    /// Se este filtro precisa ser avaliado linha a linha.
    const POR_LINHA: bool;

    /// Registra os componentes envolvidos e devolve o estado do filtro.
    fn init_state(components: &mut Components) -> Self::State;

    /// Declara o que o filtro le.
    ///
    /// `With` e `Without` nao declaram nada: so olham a assinatura do
    /// archetype. `Changed` e `Added` declaram leitura do componente, porque
    /// leem a coluna de ticks — que um sistema escritor escreve (D10).
    fn access(state: &Self::State, out: &mut Access) {
        let _ = (state, out);
    }

    /// Indica se o archetype passa.
    fn matches(state: &Self::State, archetype: &Archetype) -> bool;

    /// Resolve os ponteiros deste archetype.
    ///
    /// # Safety
    ///
    /// `archetype` precisa ter sido aceito por [`matches`](QueryFilter::matches).
    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w>;

    /// Indica se a linha passa.
    ///
    /// # Safety
    ///
    /// `row` precisa estar dentro do archetype passado a `init_fetch`.
    unsafe fn filter_fetch<'w>(fetch: Self::Fetch<'w>, row: usize, ticks: SystemTicks) -> bool;
}

// SAFETY: nao le nada e aceita tudo.
unsafe impl QueryFilter for () {
    type State = ();
    type Fetch<'w> = ();
    const POR_LINHA: bool = false;

    fn init_state(_: &mut Components) -> Self::State {}

    fn matches(_: &Self::State, _: &Archetype) -> bool {
        true
    }

    unsafe fn init_fetch<'w>(_: &Self::State, _: &'w Archetype) -> Self::Fetch<'w> {}

    unsafe fn filter_fetch<'w>(_: Self::Fetch<'w>, _: usize, _: SystemTicks) -> bool {
        true
    }
}

/// Exige a presenca de `T` sem le-lo.
///
/// Serve a componentes-marcador, que nao carregam dado: `With<Viva>` filtra sem
/// obrigar a query a devolver um `&Viva` que ninguem usaria.
pub struct With<T>(PhantomData<fn() -> T>);

// SAFETY: so consulta a assinatura do archetype; nao le dado nenhum.
unsafe impl<T: Component> QueryFilter for With<T> {
    type State = ComponentId;
    type Fetch<'w> = ();
    const POR_LINHA: bool = false;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(_: &Self::State, _: &'w Archetype) -> Self::Fetch<'w> {}

    unsafe fn filter_fetch<'w>(_: Self::Fetch<'w>, _: usize, _: SystemTicks) -> bool {
        true
    }
}

/// Exige a ausencia de `T`.
pub struct Without<T>(PhantomData<fn() -> T>);

// SAFETY: so consulta a assinatura do archetype; nao le dado nenhum.
unsafe impl<T: Component> QueryFilter for Without<T> {
    type State = ComponentId;
    type Fetch<'w> = ();
    const POR_LINHA: bool = false;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        !archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(_: &Self::State, _: &'w Archetype) -> Self::Fetch<'w> {}

    unsafe fn filter_fetch<'w>(_: Self::Fetch<'w>, _: usize, _: SystemTicks) -> bool {
        true
    }
}

/// Aceita apenas entidades cujo `T` foi exposto para escrita desde a ultima
/// execucao de quem pergunta.
///
/// **Exposto para escrita, nao necessariamente alterado.** Entregar `&mut T`
/// carimba a linha, tenha o valor mudado ou nao; comparar valores custaria mais
/// do que o filtro economiza. Ver a D10 em `docs/DECISOES.md`.
///
/// Inserir tambem conta como alterar, entao `Added<T>` implica `Changed<T>`.
pub struct Changed<T>(PhantomData<fn() -> T>);

// SAFETY: le a coluna de ticks de `T` e declara essa leitura no `access`, o que
// impede o scheduler de rodar em paralelo com quem escreve `T`.
unsafe impl<T: Component> QueryFilter for Changed<T> {
    type State = ComponentId;
    type Fetch<'w> = TicksBase<'w>;
    const POR_LINHA: bool = true;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn access(state: &Self::State, out: &mut Access) {
        out.add_component_read(*state);
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w> {
        let i = archetype.column_index(*state).expect("matches aceitou este archetype");
        TicksBase { base: archetype.ticks_base(i), _marker: PhantomData }
    }

    unsafe fn filter_fetch<'w>(fetch: Self::Fetch<'w>, row: usize, ticks: SystemTicks) -> bool {
        // SAFETY: `row` e valido pelo contrato, e o acesso declarado impede
        // escrita concorrente sobre esta coluna.
        unsafe { fetch.ler(row) }.is_changed(ticks.last_run, ticks.atual)
    }
}

/// Aceita apenas entidades que ganharam `T` desde a ultima execucao de quem
/// pergunta.
pub struct Added<T>(PhantomData<fn() -> T>);

// SAFETY: como em [`Changed`].
unsafe impl<T: Component> QueryFilter for Added<T> {
    type State = ComponentId;
    type Fetch<'w> = TicksBase<'w>;
    const POR_LINHA: bool = true;

    fn init_state(components: &mut Components) -> ComponentId {
        components.register::<T>()
    }

    fn access(state: &Self::State, out: &mut Access) {
        out.add_component_read(*state);
    }

    fn matches(state: &ComponentId, archetype: &Archetype) -> bool {
        archetype.contains(*state)
    }

    unsafe fn init_fetch<'w>(state: &Self::State, archetype: &'w Archetype) -> Self::Fetch<'w> {
        let i = archetype.column_index(*state).expect("matches aceitou este archetype");
        TicksBase { base: archetype.ticks_base(i), _marker: PhantomData }
    }

    unsafe fn filter_fetch<'w>(fetch: Self::Fetch<'w>, row: usize, ticks: SystemTicks) -> bool {
        // SAFETY: como em `Changed`.
        unsafe { fetch.ler(row) }.is_added(ticks.last_run, ticks.atual)
    }
}

macro_rules! impl_query_filter_tupla {
    ($(($F:ident, $i:tt)),+) => {
        // SAFETY: cada elemento cumpre o proprio contrato; a tupla so encaminha.
        unsafe impl<$($F: QueryFilter),+> QueryFilter for ($($F,)+) {
            type State = ($($F::State,)+);
            type Fetch<'w> = ($($F::Fetch<'w>,)+);
            const POR_LINHA: bool = $($F::POR_LINHA)||+;

            fn init_state(components: &mut Components) -> Self::State {
                ($($F::init_state(components),)+)
            }

            fn access(state: &Self::State, out: &mut Access) {
                $($F::access(&state.$i, out);)+
            }

            fn matches(state: &Self::State, archetype: &Archetype) -> bool {
                $($F::matches(&state.$i, archetype))&&+
            }

            unsafe fn init_fetch<'w>(
                state: &Self::State,
                archetype: &'w Archetype,
            ) -> Self::Fetch<'w> {
                // SAFETY: `matches` aceitou, logo aceitou para cada elemento.
                unsafe { ($($F::init_fetch(&state.$i, archetype),)+) }
            }

            unsafe fn filter_fetch<'w>(
                fetch: Self::Fetch<'w>,
                row: usize,
                ticks: SystemTicks,
            ) -> bool {
                // SAFETY: `row` valido pelo contrato, para todos os elementos.
                unsafe { $($F::filter_fetch(fetch.$i, row, ticks))&&+ }
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
    filtro: F::State,
    ticks: SystemTicks,
    correspondentes: Vec<ArchetypeId>,
    proximo: usize,
    fetch: Option<D::Fetch<'w>>,
    fetch_filtro: Option<F::Fetch<'w>>,
    row: usize,
    len: usize,
    /// Quantas entidades os archetypes correspondentes contem. E exato quando
    /// nenhum filtro avalia por linha, e um limite superior quando algum avalia.
    restantes: usize,
}

impl<'w, D: QueryData, F: QueryFilter> QueryIter<'w, D, F> {
    /// Monta a query, resolvendo os archetypes que casam.
    ///
    /// # Panics
    ///
    /// Se `D` pedir o mesmo componente de forma conflitante.
    pub(crate) fn new(
        archetypes: &'w Archetypes,
        components: &mut Components,
        ticks: SystemTicks,
    ) -> Self {
        let estado = QueryState::<D, F>::new(components);
        Self::from_state(archetypes, &estado, ticks)
    }

    /// Monta a query a partir de um estado ja resolvido.
    ///
    /// E o caminho de um sistema, que resolve o estado uma vez na
    /// inicializacao e o reaproveita a cada passo.
    pub(crate) fn from_state(
        archetypes: &'w Archetypes,
        estado: &QueryState<D, F>,
        ticks: SystemTicks,
    ) -> Self {
        let state = estado.data.clone();
        let filtro = estado.filtro.clone();

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
            filtro,
            ticks,
            correspondentes,
            proximo: 0,
            fetch: None,
            fetch_filtro: None,
            row: 0,
            len: 0,
            restantes,
        }
    }

    /// Limite superior de quantas entidades a iteracao pode devolver.
    ///
    /// Exato quando nenhum filtro avalia por linha. Com `Changed` ou `Added`, o
    /// numero real so se sabe percorrendo.
    #[must_use]
    pub const fn upper_bound(&self) -> usize {
        self.restantes
    }
}

impl<'w, D: QueryData, F: QueryFilter> Iterator for QueryIter<'w, D, F> {
    type Item = D::Item<'w>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.row < self.len {
                let row = self.row;
                self.row += 1;
                self.restantes -= 1;

                // Filtros por archetype ja foram decididos na construcao; so os
                // por linha custam alguma coisa aqui, e o `const` deixa o
                // compilador apagar o desvio quando nao ha nenhum.
                if F::POR_LINHA {
                    let ff = self.fetch_filtro.expect("ha linhas, logo ha fetch de filtro");
                    // SAFETY: `row < len`, com `len` vindo do archetype de onde
                    // `ff` foi construido.
                    if !unsafe { F::filter_fetch(ff, row, self.ticks) } {
                        continue;
                    }
                }

                let fetch = self.fetch.expect("ha linhas, logo ha fetch");
                // SAFETY: `row < len`, e `len` e o numero de entidades do
                // archetype de onde `fetch` foi construido. Cada linha e
                // visitada uma unica vez, ja que `row` so cresce.
                return Some(unsafe { D::fetch(fetch, row) });
            }

            let id = *self.correspondentes.get(self.proximo)?;
            self.proximo += 1;

            let archetypes: &'w Archetypes = self.archetypes;
            let archetype = archetypes.get(id).expect("id veio desta colecao");
            // SAFETY: `id` so entrou em `correspondentes` depois de `matches`
            // aceitar este archetype, tanto para os dados quanto para o filtro.
            unsafe {
                self.fetch = Some(D::init_fetch(&self.state, archetype, self.ticks));
                self.fetch_filtro = Some(F::init_fetch(&self.filtro, archetype));
            }
            self.row = 0;
            self.len = archetype.len();
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // Com filtro por linha nao ha piso: todas as entidades podem ser
        // recusadas. Sem ele, o limite superior tambem e o exato.
        let piso = if F::POR_LINHA { 0 } else { self.restantes };
        (piso, Some(self.restantes))
    }
}

#[cfg(test)]
mod tests {
    use crate::{Added, Changed, Entity, QueryFilter, With, Without, World};

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

        assert_eq!(w.query::<&Posicao>().upper_bound(), 7);
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

    // ---------------------------------------------------- deteccao de mudanca --

    #[test]
    fn changed_seleciona_so_quem_foi_exposto_para_escrita() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.get_mut::<Posicao>(a).unwrap().0 = 10.0;

        let vistos: Vec<_> =
            w.query_filtered_since::<&Posicao, Changed<Posicao>>(marco).map(|p| p.0).collect();

        assert_eq!(vistos, vec![10.0]);
    }

    #[test]
    fn escrever_pela_query_marca_a_linha() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        for p in w.query::<&mut Posicao>() {
            if p.0 > 1.5 {
                p.0 += 100.0;
            }
        }

        // Toda linha visitada foi carimbada, tenha o valor mudado ou nao: o
        // contrato e "exposto para escrita".
        assert_eq!(w.query_filtered_since::<&Posicao, Changed<Posicao>>(marco).count(), 2);
    }

    #[test]
    fn query_mutavel_com_filtro_de_mudanca_e_permitida() {
        // O padrao mais comum de todos. O filtro le a coluna de ticks, nao os
        // valores, entao nao conflita com a escrita da propria query.
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.get_mut::<Posicao>(a).unwrap().0 = 5.0;

        w.increment_change_tick();
        let mut n = 0;
        for p in w.query_filtered_since::<&mut Posicao, Changed<Posicao>>(marco) {
            p.0 *= 2.0;
            n += 1;
        }

        assert_eq!(n, 1);
        assert_eq!(w.get::<Posicao>(a).map(|p| p.0), Some(10.0));
    }

    #[test]
    fn added_seleciona_so_quem_nasceu_depois() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.spawn((Posicao(2.0),));

        let vistos: Vec<_> =
            w.query_filtered_since::<&Posicao, Added<Posicao>>(marco).map(|p| p.0).collect();

        assert_eq!(vistos, vec![2.0]);
    }

    #[test]
    fn alterar_nao_faz_o_componente_contar_como_adicionado() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.get_mut::<Posicao>(a).unwrap().0 = 9.0;

        assert_eq!(w.query_filtered_since::<&Posicao, Added<Posicao>>(marco).count(), 0);
        assert_eq!(w.query_filtered_since::<&Posicao, Changed<Posicao>>(marco).count(), 1);
    }

    #[test]
    fn nada_mudou_desde_agora() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));
        let agora = w.change_tick();

        assert_eq!(w.query_filtered_since::<&Posicao, Changed<Posicao>>(agora).count(), 0);
    }

    #[test]
    fn filtro_de_mudanca_combina_com_filtro_de_archetype() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0), Viva));
        let b = w.spawn((Posicao(2.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.get_mut::<Posicao>(a).unwrap().0 = 10.0;
        w.get_mut::<Posicao>(b).unwrap().0 = 20.0;

        let vistos: Vec<_> = w
            .query_filtered_since::<&Posicao, (With<Viva>, Changed<Posicao>)>(marco)
            .map(|p| p.0)
            .collect();

        assert_eq!(vistos, vec![10.0], "os dois mudaram, mas so um esta vivo");
    }

    // `POR_LINHA` e constante, entao estas verificacoes acontecem na compilacao
    // e nao na execucao — o que e o ponto: e justamente por ser `const` que o
    // iterador consegue apagar o desvio por linha no caso comum. Um `assert!`
    // dentro de `#[test]` sobre valor constante o clippy rejeita, com razao.
    const _: () = assert!(!<() as QueryFilter>::POR_LINHA);
    const _: () = assert!(!<With<Viva> as QueryFilter>::POR_LINHA);
    const _: () = assert!(!<(With<Viva>, Without<Congelada>) as QueryFilter>::POR_LINHA);
    const _: () = assert!(<Changed<Posicao> as QueryFilter>::POR_LINHA);
    const _: () = assert!(<Added<Posicao> as QueryFilter>::POR_LINHA);
    const _: () = assert!(<(With<Viva>, Changed<Posicao>) as QueryFilter>::POR_LINHA);

    #[test]
    fn size_hint_com_filtro_por_linha_nao_promete_piso() {
        let mut w = World::new();
        for i in 0..5 {
            w.spawn((Posicao(i as f32),));
        }
        let agora = w.change_tick();

        let iter = w.query_filtered_since::<&Posicao, Changed<Posicao>>(agora);
        // Nenhuma mudou desde `agora`, mas o teto continua sendo o total.
        assert_eq!(iter.size_hint(), (0, Some(5)));
    }

    #[test]
    fn migrar_de_archetype_nao_marca_como_alterado() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0),));
        let marco = w.change_tick();

        w.increment_change_tick();
        w.insert(a, (Viva,));

        assert_eq!(
            w.query_filtered_since::<&Posicao, Changed<Posicao>>(marco).count(),
            0,
            "a posicao foi carregada para o novo archetype, nao tocada"
        );
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

/// Estado resolvido de uma query, reaproveitavel entre execucoes.
///
/// Um sistema resolve isto uma vez, na inicializacao, e o usa a cada passo. Sem
/// ele, cada execucao repetiria o registro dos componentes e o calculo do
/// acesso.
#[derive(Debug, Clone)]
pub struct QueryState<D: QueryData, F: QueryFilter = ()> {
    data: D::State,
    filtro: F::State,
    acesso: Access,
}

impl<D: QueryData, F: QueryFilter> QueryState<D, F> {
    /// Resolve o estado, registrando no mundo os componentes envolvidos.
    ///
    /// # Panics
    ///
    /// Se `D` pedir o mesmo componente de forma conflitante consigo mesma.
    pub fn new(components: &mut Components) -> Self {
        let data = D::init_state(components);
        let filtro = F::init_state(components);

        // A checagem de autoconflito olha so o que a query le e escreve de
        // verdade. O acesso do filtro entra depois, de proposito: `Changed<T>`
        // le a coluna de ticks de `T`, nao os valores, entao
        // `Query<&mut T, Changed<T>>` — o padrao mais comum de todos — nao e um
        // conflito. Somar o filtro antes o rejeitaria por engano.
        //
        // Para o scheduler, porem, essa leitura conta: outro sistema escrevendo
        // `T` escreve os ticks. Por isso ela entra no acesso reportado.
        let mut acesso = Access::new();
        D::access(&data, &mut acesso);
        if let Some(Conflict::Component(id)) = acesso.self_conflict() {
            let nome = components.info(id).map_or("?", |i| i.name());
            panic!("query pede o componente {nome} de forma conflitante consigo mesma");
        }
        F::access(&filtro, &mut acesso);

        Self { data, filtro, acesso }
    }

    /// Componentes que esta query le e escreve.
    #[inline]
    #[must_use]
    pub const fn access(&self) -> &Access {
        &self.acesso
    }
}

/// Query pronta para uso, como um sistema a recebe.
///
/// Guarda o mundo e o estado ja resolvido; `iter` produz a iteracao.
pub struct Query<'w, 's, D: QueryData, F: QueryFilter = ()> {
    archetypes: &'w Archetypes,
    estado: &'s QueryState<D, F>,
    ticks: SystemTicks,
}

impl<'w, 's, D: QueryData, F: QueryFilter> Query<'w, 's, D, F> {
    /// Constroi a query a partir dos archetypes, do estado e dos ticks do
    /// sistema que a recebe.
    #[must_use]
    pub const fn new(
        archetypes: &'w Archetypes,
        estado: &'s QueryState<D, F>,
        ticks: SystemTicks,
    ) -> Self {
        Self { archetypes, estado, ticks }
    }

    /// Percorre as entidades que casam.
    ///
    /// Recebe `&mut self` de proposito: com `&self`, duas chamadas produziriam
    /// dois iteradores vivos ao mesmo tempo, e num query com `&mut T` isso seria
    /// aliasing sobre a mesma linha.
    pub fn iter(&mut self) -> QueryIter<'_, D, F> {
        QueryIter::from_state(self.archetypes, self.estado, self.ticks)
    }

    /// Quantas entidades casam neste instante.
    ///
    /// Com filtro por linha, percorre para contar; sem, e imediato.
    #[must_use]
    pub fn count(&self) -> usize {
        let iter = QueryIter::<D, F>::from_state(self.archetypes, self.estado, self.ticks);
        if F::POR_LINHA { iter.count() } else { iter.upper_bound() }
    }

    /// Indica se nenhuma entidade casa.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.count() == 0
    }
}

impl<D: QueryData, F: QueryFilter> std::fmt::Debug for Query<'_, '_, D, F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Query").field("correspondencias", &self.count()).finish()
    }
}
