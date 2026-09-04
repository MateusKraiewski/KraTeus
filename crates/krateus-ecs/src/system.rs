//! Sistemas: funcoes que o scheduler executa sobre o mundo.
//!
//! Um sistema e uma funcao comum cujos parametros dizem de que ele precisa. Os
//! tipos dos parametros sao a **declaracao de acesso**: e deles que o scheduler
//! deriva o que pode rodar em paralelo, sem que ninguem ordene sistemas a mao
//! (secao 5 do documento de visao).
//!
//! ```
//! use krateus_ecs::{IntoSystem, Query, Res, ResMut, System, Tick, World, WorldCell};
//!
//! struct Posicao(f32);
//! struct Velocidade(f32);
//! struct Passo(f32);
//! struct Contador(u32);
//!
//! fn integrar(mut q: Query<(&mut Posicao, &Velocidade)>, passo: Res<Passo>) {
//!     for (p, v) in q.iter() {
//!         p.0 += v.0 * passo.0;
//!     }
//! }
//!
//! fn contar(mut c: ResMut<Contador>) {
//!     c.0 += 1;
//! }
//!
//! let mut world = World::new();
//! world.insert_resource(Passo(0.5));
//! world.insert_resource(Contador(0));
//! world.spawn((Posicao(0.0), Velocidade(4.0)));
//!
//! let mut s1 = IntoSystem::into_system(integrar);
//! let mut s2 = IntoSystem::into_system(contar);
//! s1.initialize(&mut world);
//! s2.initialize(&mut world);
//!
//! // Os dois nao conflitam: um escreve Posicao, o outro escreve Contador.
//! assert!(s1.access().compatible_with(s2.access()));
//!
//! // Ticks distintos e crescentes: e o scheduler que os distribui na pratica,
//! // a partir da posicao de cada sistema na ordem deterministica.
//! let cell = WorldCell::new(&mut world);
//! // SAFETY: execucao sequencial, uma de cada vez.
//! unsafe {
//!     s1.run(cell, Tick::new(1));
//!     s2.run(cell, Tick::new(2));
//! }
//!
//! assert_eq!(world.query::<&Posicao>().next().map(|p| p.0), Some(2.0));
//! assert_eq!(world.resource::<Contador>().0, 1);
//! ```

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use crate::access::Access;
use crate::commands::Commands;
use crate::query::{Query, QueryData, QueryFilter, QueryState};
use crate::resource::{Resource, ResourceId};
use crate::tick::{SystemTicks, Tick};
use crate::world::World;
use crate::world_cell::WorldCell;

/// Algo que um sistema pede como parametro.
///
/// Como em [`QueryData`], o tempo de vida fica nos tipos associados e nao no
/// trait, para que [`State`](SystemParam::State) possa ser guardado pelo
/// sistema entre execucoes. Nas implementacoes, as assinaturas precisam usar a
/// forma projetada `Self::Item<'w, 's>`; escrever o tipo concreto produz
/// `E0195`.
///
/// # Safety
///
/// [`access`](SystemParam::access) precisa declarar **exatamente** o que
/// [`fetch`](SystemParam::fetch) alcanca. O scheduler confia nessa declaracao
/// para decidir o que roda junto: um acesso omitido vira corrida de dados.
pub unsafe trait SystemParam {
    /// Dados resolvidos uma vez, na inicializacao.
    type State: Send + Sync + 'static;
    /// O que o sistema recebe. `'w` e o mundo; `'s`, o estado.
    type Item<'w, 's>;

    /// Resolve o estado, registrando no mundo os tipos envolvidos.
    fn init(world: &mut World) -> Self::State;

    /// Declara o que sera lido e escrito.
    fn access(state: &Self::State, out: &mut Access);

    /// Aplica ao mundo o que o parametro acumulou durante a execucao.
    ///
    /// So o command buffer usa; os demais nao tem nada a aplicar.
    fn apply(state: &mut Self::State, world: &mut World) {
        let _ = (state, world);
    }

    /// Monta o parametro.
    ///
    /// # Safety
    ///
    /// Nenhum acesso que conflite com o declarado em
    /// [`access`](SystemParam::access) pode estar ativo.
    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        world: WorldCell<'w>,
        ticks: SystemTicks,
    ) -> Self::Item<'w, 's>;
}

// ---------------------------------------------------------------- Commands --

// SAFETY: a fila nao toca no mundo durante a execucao; o que ela enfileira e
// aplicado depois, sequencialmente. Por isso nao declara acesso nenhum.
unsafe impl SystemParam for &mut Commands {
    type State = Commands;
    type Item<'w, 's> = &'s mut Commands;

    fn init(_: &mut World) -> Self::State {
        Commands::new()
    }

    fn access(_: &Self::State, _: &mut Access) {}

    fn apply(state: &mut Self::State, world: &mut World) {
        state.apply(world);
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        _: WorldCell<'w>,
        _: SystemTicks,
    ) -> Self::Item<'w, 's> {
        state
    }
}

// --------------------------------------------------------------------- Res --

/// Leitura de um recurso.
pub struct Res<'w, R: Resource> {
    valor: &'w R,
}

impl<R: Resource> Deref for Res<'_, R> {
    type Target = R;

    fn deref(&self) -> &R {
        self.valor
    }
}

impl<R: Resource + std::fmt::Debug> std::fmt::Debug for Res<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.valor.fmt(f)
    }
}

// SAFETY: declara leitura do recurso e so entrega `&R`.
unsafe impl<'a, R: Resource> SystemParam for Res<'a, R> {
    /// O identificador do recurso, resolvido na inicializacao. Guardar evita
    /// consultar o registro a cada execucao.
    type State = ResourceId;
    type Item<'w, 's> = Res<'w, R>;

    fn init(world: &mut World) -> Self::State {
        world.resources_mut().register::<R>()
    }

    fn access(state: &Self::State, out: &mut Access) {
        out.add_resource_read(*state);
    }

    unsafe fn fetch<'w, 's>(
        _: &'s mut Self::State,
        world: WorldCell<'w>,
        _: SystemTicks,
    ) -> Self::Item<'w, 's> {
        // SAFETY: o contrato garante que nenhuma escrita a `R` esta ativa.
        let mundo = unsafe { world.world() };
        let valor = mundo.get_resource::<R>().unwrap_or_else(|| {
            panic!("sistema pede Res<{}>, que nao foi inserido", std::any::type_name::<R>())
        });
        Res { valor }
    }
}

// ------------------------------------------------------------------ ResMut --

/// Escrita de um recurso.
pub struct ResMut<'w, R: Resource> {
    valor: &'w mut R,
}

impl<R: Resource> Deref for ResMut<'_, R> {
    type Target = R;

    fn deref(&self) -> &R {
        self.valor
    }
}

impl<R: Resource> DerefMut for ResMut<'_, R> {
    fn deref_mut(&mut self) -> &mut R {
        self.valor
    }
}

impl<R: Resource + std::fmt::Debug> std::fmt::Debug for ResMut<'_, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.valor.fmt(f)
    }
}

// SAFETY: declara escrita do recurso. A referencia mutavel vem de
// `Resources::get_unchecked_mut`, cujo contrato o scheduler cumpre ao nao
// deixar rodar junto nada que toque o mesmo recurso.
unsafe impl<'a, R: Resource> SystemParam for ResMut<'a, R> {
    /// Como em [`Res`], o identificador resolvido na inicializacao.
    type State = ResourceId;
    type Item<'w, 's> = ResMut<'w, R>;

    fn init(world: &mut World) -> Self::State {
        world.resources_mut().register::<R>()
    }

    fn access(state: &Self::State, out: &mut Access) {
        out.add_resource_write(*state);
    }

    unsafe fn fetch<'w, 's>(
        _: &'s mut Self::State,
        world: WorldCell<'w>,
        _: SystemTicks,
    ) -> Self::Item<'w, 's> {
        // SAFETY: o contrato de `fetch` garante que nada conflitante com o
        // acesso declarado esta ativo.
        let mundo = unsafe { world.world() };

        // SAFETY: o mesmo contrato garante que este recurso nao esta sendo
        // acessado por mais ninguem — foi essa a condicao para o scheduler
        // deixar este sistema rodar agora.
        let ptr = unsafe { mundo.resources().get_ptr::<R>() }.unwrap_or_else(|| {
            panic!("sistema pede ResMut<{}>, que nao foi inserido", std::any::type_name::<R>())
        });

        // SAFETY: o ponteiro veio do slot vivo de `R` e a exclusividade e a
        // condicao acima. E aqui, e nao dentro de `get_ptr`, que a afirmacao de
        // acesso exclusivo pode ser sustentada.
        ResMut { valor: unsafe { &mut *ptr.as_ptr() } }
    }
}

// ------------------------------------------------------------------- Query --

// SAFETY: o acesso vem do proprio `QueryState`, que o calcula a partir de `D`.
unsafe impl<'a, 'b, D: QueryData + 'static, F: QueryFilter + 'static> SystemParam
    for Query<'a, 'b, D, F>
{
    type State = QueryState<D, F>;
    type Item<'w, 's> = Query<'w, 's, D, F>;

    fn init(world: &mut World) -> Self::State {
        QueryState::new(world.components_mut())
    }

    fn access(state: &Self::State, out: &mut Access) {
        out.extend(state.access());
    }

    unsafe fn fetch<'w, 's>(
        state: &'s mut Self::State,
        world: WorldCell<'w>,
        ticks: SystemTicks,
    ) -> Self::Item<'w, 's> {
        // SAFETY: o contrato garante que nada conflitante esta ativo. A query
        // so precisa de acesso compartilhado a estrutura dos archetypes; as
        // referencias mutaveis a componentes vem da proveniencia das colunas.
        let mundo = unsafe { world.world() };
        Query::new(mundo.archetypes(), state, ticks)
    }
}

// ------------------------------------------------------------------ Tuplas --

macro_rules! impl_system_param_tupla {
    ($(($P:ident, $i:tt)),*) => {
        // SAFETY: cada elemento cumpre o proprio contrato; a tupla so encaminha.
        unsafe impl<$($P: SystemParam),*> SystemParam for ($($P,)*) {
            type State = ($($P::State,)*);
            type Item<'w, 's> = ($($P::Item<'w, 's>,)*);

            fn init(world: &mut World) -> Self::State {
                ($($P::init(world),)*)
            }

            fn access(state: &Self::State, out: &mut Access) {
                $($P::access(&state.$i, out);)*
            }

            fn apply(state: &mut Self::State, world: &mut World) {
                $($P::apply(&mut state.$i, world);)*
            }

            unsafe fn fetch<'w, 's>(
                state: &'s mut Self::State,
                world: WorldCell<'w>,
                ticks: SystemTicks,
            ) -> Self::Item<'w, 's> {
                // SAFETY: o contrato foi cumprido por quem chamou, e os
                // parametros de um mesmo sistema tem acessos compativeis entre
                // si — verificado na construcao do sistema.
                unsafe { ($($P::fetch(&mut state.$i, world, ticks),)*) }
            }
        }
    };
}

// SAFETY: sem parametros, nao ha acesso nem estado.
unsafe impl SystemParam for () {
    type State = ();
    type Item<'w, 's> = ();

    fn init(_: &mut World) -> Self::State {}
    fn access(_: &Self::State, _: &mut Access) {}
    unsafe fn fetch<'w, 's>(
        _: &'s mut Self::State,
        _: WorldCell<'w>,
        _: SystemTicks,
    ) -> Self::Item<'w, 's> {
    }
}

impl_system_param_tupla!((A, 0));
impl_system_param_tupla!((A, 0), (B, 1));
impl_system_param_tupla!((A, 0), (B, 1), (C, 2));
impl_system_param_tupla!((A, 0), (B, 1), (C, 2), (D, 3));
impl_system_param_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_system_param_tupla!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));

// ------------------------------------------------------------------ System --

/// Unidade de trabalho que o scheduler executa.
pub trait System: Send + Sync + 'static {
    /// Nome do sistema, para diagnostico e relatorio de conflito.
    fn name(&self) -> &'static str;

    /// Resolve os parametros contra o mundo. Precisa preceder qualquer `run`.
    fn initialize(&mut self, world: &mut World);

    /// O que este sistema le e escreve.
    ///
    /// Vazio antes de [`initialize`](System::initialize).
    fn access(&self) -> &Access;

    /// Executa o sistema.
    ///
    /// # Safety
    ///
    /// Nenhum outro sistema com acesso conflitante pode estar em execucao, e
    /// [`initialize`](System::initialize) precisa ter sido chamado.
    ///
    /// `atual` e o tick desta execucao, atribuido pelo scheduler a partir da
    /// posicao do sistema na ordem deterministica — nunca de um contador
    /// atomico, que faria os ticks gravados variarem entre execucoes (D10).
    unsafe fn run(&mut self, world: WorldCell<'_>, atual: Tick);

    /// Aplica o que os parametros acumularam — na pratica, o command buffer.
    ///
    /// Roda sequencialmente, depois da subetapa.
    fn apply_deferred(&mut self, world: &mut World);
}

/// Sistema construido a partir de uma funcao.
pub struct FunctionSystem<Func, Marker>
where
    Func: SystemParamFunction<Marker>,
{
    func: Func,
    estado: Option<<Func::Param as SystemParam>::State>,
    acesso: Access,
    /// Tick da execucao anterior deste sistema. Comeca em zero, entao na
    /// primeira execucao tudo o que existe conta como novo.
    last_run: Tick,
    _marker: PhantomData<fn() -> Marker>,
}

impl<Func, Marker> System for FunctionSystem<Func, Marker>
where
    Func: SystemParamFunction<Marker>,
    Marker: 'static,
{
    fn name(&self) -> &'static str {
        std::any::type_name::<Func>()
    }

    fn initialize(&mut self, world: &mut World) {
        let estado = <Func::Param as SystemParam>::init(world);
        let mut acesso = Access::new();
        <Func::Param as SystemParam>::access(&estado, &mut acesso);

        if let Some(conflito) = acesso.self_conflict() {
            panic!("sistema {} pede acesso conflitante consigo mesmo: {conflito:?}", self.name());
        }

        self.acesso = acesso;
        self.estado = Some(estado);
    }

    fn access(&self) -> &Access {
        &self.acesso
    }

    unsafe fn run(&mut self, world: WorldCell<'_>, atual: Tick) {
        let ticks = SystemTicks::new(self.last_run, atual);
        let estado = self.estado.as_mut().unwrap_or_else(|| {
            panic!("sistema {} nao foi inicializado", std::any::type_name::<Func>())
        });

        // SAFETY: o contrato de `System::run` transfere para quem chama a
        // garantia de que nada conflitante esta ativo.
        let param = unsafe { <Func::Param as SystemParam>::fetch(estado, world, ticks) };
        self.func.executar(param);

        self.last_run = atual;
    }

    fn apply_deferred(&mut self, world: &mut World) {
        if let Some(estado) = self.estado.as_mut() {
            <Func::Param as SystemParam>::apply(estado, world);
        }
    }
}

/// Funcao que pode virar sistema.
///
/// O parametro `Marker` existe so para dar impls distintas por aridade; ele
/// nunca aparece em valor.
pub trait SystemParamFunction<Marker>: Send + Sync + 'static {
    /// Tupla dos parametros declarados.
    type Param: SystemParam;

    /// Chama a funcao com os parametros ja montados.
    fn executar(&mut self, param: <Self::Param as SystemParam>::Item<'_, '_>);
}

/// Converte algo em [`System`].
pub trait IntoSystem<Marker> {
    /// Tipo concreto do sistema resultante.
    type System: System;

    /// Constroi o sistema.
    fn into_system(self) -> Self::System;
}

impl<Func, Marker> IntoSystem<Marker> for Func
where
    Func: SystemParamFunction<Marker>,
    Marker: 'static,
{
    type System = FunctionSystem<Func, Marker>;

    fn into_system(self) -> Self::System {
        FunctionSystem {
            func: self,
            estado: None,
            acesso: Access::new(),
            last_run: Tick::ZERO,
            _marker: PhantomData,
        }
    }
}

macro_rules! impl_system_param_function {
    ($($P:ident),*) => {
        #[allow(non_snake_case)]
        impl<Func, $($P: SystemParam),*> SystemParamFunction<fn($($P),*)> for Func
        where
            Func: Send + Sync + 'static,
            // As tres vidas sao independentes de proposito. Reaproveitar `'w`
            // para o emprestimo de `Func` amarraria o `&mut self` de `executar`
            // ao tempo de vida do mundo, e o compilador passaria a exigir que um
            // sobrevivesse ao outro.
            //
            // O primeiro bound existe para a inferencia: e ele que fixa quais
            // sao os tipos `$P` a partir da assinatura da funcao. O segundo e o
            // que a chamada realmente usa.
            for<'a, 'w, 's> &'a mut Func:
                FnMut($($P),*) + FnMut($(<$P as SystemParam>::Item<'w, 's>),*),
        {
            type Param = ($($P,)*);

            fn executar(&mut self, param: <Self::Param as SystemParam>::Item<'_, '_>) {
                // O desvio por `chamar` existe porque o compilador nao consegue
                // usar diretamente o bound com `Item`, que e uma projecao. Aqui
                // os tipos ja estao concretos.
                fn chamar<$($P),*>(mut f: impl FnMut($($P),*), $($P: $P),*) {
                    f($($P),*);
                }
                let ($($P,)*) = param;
                chamar(self, $($P),*);
            }
        }
    };
}

impl_system_param_function!();
impl_system_param_function!(A);
impl_system_param_function!(A, B);
impl_system_param_function!(A, B, C);
impl_system_param_function!(A, B, C, D);
impl_system_param_function!(A, B, C, D, E);
impl_system_param_function!(A, B, C, D, E, F);
