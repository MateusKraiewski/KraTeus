//! O container de entidades, componentes e archetypes.
//!
//! O `World` e a unica porta de entrada do armazenamento. Ele mantem tres
//! invariantes das quais todo o resto depende:
//!
//! 1. Toda entidade viva tem localizacao valida — archetype existente e linha
//!    dentro dele.
//! 2. Dentro de um archetype, todas as colunas tem o mesmo comprimento, igual
//!    ao numero de entidades.
//! 3. A entidade guardada na linha `row` e a mesma cuja localizacao aponta para
//!    `row`.
//!
//! Toda operacao que mexe em archetype restaura as tres antes de retornar,
//! inclusive corrigindo a localizacao da entidade deslocada pelo `swap_remove`.

use crate::archetype::{ArchetypeId, Archetypes, move_row};
use crate::bundle::Bundle;
use crate::component::{Component, ComponentId, Components};
use crate::entity::{Entities, Entity, EntityLocation};
use crate::query::{QueryData, QueryFilter, QueryIter};
use crate::resource::{Resource, Resources};
use crate::tick::{ComponentTicks, SystemTicks, Tick};

/// Armazenamento de entidades e componentes.
#[derive(Debug)]
pub struct World {
    entities: Entities,
    archetypes: Archetypes,
    components: Components,
    resources: Resources,
    /// Buffers reaproveitados entre chamadas para montar assinaturas sem
    /// alocar a cada `spawn` — a secao 11 do documento de visao pede
    /// explicitamente que nao se aloque por frame.
    decl: Vec<ComponentId>,
    sig: Vec<ComponentId>,
    /// Instante logico corrente. Avanca uma vez por sistema executado; ver a
    /// D10 em `docs/DECISOES.md`.
    change_tick: Tick,
}

impl Default for World {
    fn default() -> Self {
        // Manual: o `change_tick` comeca em `PRIMEIRO`, e nao no `Default` de
        // `Tick`, que e o sentinela de "nunca".
        Self::new()
    }
}

impl World {
    /// Cria um mundo vazio.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entities: Entities::new(),
            archetypes: Archetypes::new(),
            components: Components::new(),
            resources: Resources::new(),
            decl: Vec::new(),
            sig: Vec::new(),
            change_tick: Tick::PRIMEIRO,
        }
    }

    /// Quantidade de entidades vivas.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entities.len()
    }

    /// Indica se nao ha entidades vivas.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Tabela de entidades.
    #[inline]
    #[must_use]
    pub const fn entities(&self) -> &Entities {
        &self.entities
    }

    /// Archetypes existentes.
    #[inline]
    #[must_use]
    pub const fn archetypes(&self) -> &Archetypes {
        &self.archetypes
    }

    /// Registro de tipos de componente.
    #[inline]
    #[must_use]
    pub const fn components(&self) -> &Components {
        &self.components
    }

    /// Registro de tipos de componente, para registrar tipos novos.
    ///
    /// Exposto para os parametros de sistema, que resolvem seus componentes na
    /// inicializacao.
    #[inline]
    pub const fn components_mut(&mut self) -> &mut Components {
        &mut self.components
    }

    /// Recursos do mundo, para registrar tipos novos.
    #[inline]
    pub const fn resources_mut(&mut self) -> &mut Resources {
        &mut self.resources
    }

    /// Instante logico corrente.
    #[inline]
    #[must_use]
    pub const fn change_tick(&self) -> Tick {
        self.change_tick
    }

    /// Avanca o instante logico e devolve o novo valor.
    ///
    /// Chamado pelo scheduler, uma vez por sistema executado.
    pub const fn increment_change_tick(&mut self) -> Tick {
        self.change_tick = self.change_tick.adiantado(1);
        self.change_tick
    }

    /// Posiciona o instante logico. Usado pelo scheduler ao distribuir ticks.
    pub const fn set_change_tick(&mut self, tick: Tick) {
        self.change_tick = tick;
    }

    /// Ticks de insercao e alteracao do componente `T` da entidade.
    #[must_use]
    pub fn component_ticks<T: Component>(&self, entity: Entity) -> Option<ComponentTicks> {
        let (cid, loc) = self.localiza_coluna::<T>(entity)?;
        self.archetypes.get(loc.archetype)?.component_ticks(cid, loc.row as usize)
    }

    /// Indica se `T` foi inserido na entidade depois de `last_run`.
    #[must_use]
    pub fn is_added<T: Component>(&self, entity: Entity, last_run: Tick) -> bool {
        self.component_ticks::<T>(entity).is_some_and(|t| t.is_added(last_run, self.change_tick))
    }

    /// Indica se `T` foi exposto para escrita depois de `last_run`.
    #[must_use]
    pub fn is_changed<T: Component>(&self, entity: Entity, last_run: Tick) -> bool {
        self.component_ticks::<T>(entity).is_some_and(|t| t.is_changed(last_run, self.change_tick))
    }

    /// Puxa para a idade maxima todo tick velho demais para a comparacao
    /// circular.
    ///
    /// Sem isso, um componente que passa muito tempo sem ser tocado volta a
    /// parecer recente depois da virada do contador. O gatilho e a contagem de
    /// ticks, nunca tempo de relogio: um gatilho temporal faria a mesma
    /// simulacao divergir entre maquinas de velocidades diferentes (D09).
    ///
    /// Devolve quantas instancias de componente foram ajustadas — nao quantos
    /// ticks, ja que `added` e `changed` de uma mesma instancia contam junto.
    pub fn saneia_ticks(&mut self) -> usize {
        let agora = self.change_tick;
        let mut ajustados = 0;
        for i in 0..self.archetypes.len() {
            if let Some(arch) = self.archetypes.get_mut(ArchetypeId::from_index(i)) {
                ajustados += arch.saneia_ticks(agora);
            }
        }
        ajustados
    }

    /// Indica se a entidade existe.
    #[inline]
    #[must_use]
    pub fn contains(&self, entity: Entity) -> bool {
        self.entities.contains(entity)
    }

    /// Cria uma entidade com os componentes do bundle.
    ///
    /// # Panics
    ///
    /// Se o bundle declarar o mesmo tipo de componente mais de uma vez.
    pub fn spawn<B: Bundle>(&mut self, bundle: B) -> Entity {
        let (mut decl, mut sig) = self.pega_buffers();

        B::register(&mut self.components, &mut decl);
        sig.extend_from_slice(&decl);
        let archetype = self.archetypes.get_or_insert(&mut sig, &self.components);
        assert_eq!(decl.len(), sig.len(), "bundle com componente repetido");

        let entity = self.entities.alloc();
        let tick = self.change_tick;
        let arch = self.archetypes.get_mut(archetype).expect("archetype recem-obtido");
        let row = arch.allocate(entity, tick);

        bundle.take(&mut |i, ptr| {
            let cid = decl[i];
            let coluna = arch.column_mut(cid).expect("coluna presente na assinatura");
            // SAFETY: `ptr` aponta para o componente `cid`, cuja posse o
            // contrato de `Bundle::take` transfere para ca; `coluna` guarda
            // exatamente esse tipo. Cada posicao e visitada uma unica vez, e a
            // ausencia de repeticao foi verificada acima, entao nenhuma coluna
            // recebe dois valores para a mesma entidade.
            unsafe { coluna.push_from(ptr) };
        });

        self.entities.set_location(entity, EntityLocation { archetype, row: row as u32 });

        self.devolve_buffers(decl, sig);
        entity
    }

    /// Remove a entidade e destroi seus componentes.
    ///
    /// Devolve `false` se a entidade ja nao existia.
    pub fn despawn(&mut self, entity: Entity) -> bool {
        if !self.entities.contains(entity) {
            return false;
        }

        if let Some(loc) = self.entities.free(entity) {
            let arch = self.archetypes.get_mut(loc.archetype).expect("archetype da entidade");
            if let Some(deslocada) = arch.swap_remove(loc.row as usize) {
                // A entidade que estava na ultima linha ocupou o buraco.
                self.entities.set_location(deslocada, loc);
            }
        }
        true
    }

    /// Indica se a entidade tem o componente `T`.
    #[must_use]
    pub fn has<T: Component>(&self, entity: Entity) -> bool {
        self.localiza_coluna::<T>(entity).is_some()
    }

    /// Referencia ao componente `T` da entidade.
    #[must_use]
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        let (cid, loc) = self.localiza_coluna::<T>(entity)?;
        let coluna = self.archetypes.get(loc.archetype)?.column(cid)?;
        // SAFETY: `localiza_coluna` confirmou que o archetype tem a coluna de
        // `T` e que a entidade esta viva na linha `loc.row`. O invariante de
        // colunas do mesmo comprimento garante `loc.row < coluna.len()`.
        Some(unsafe { coluna.get::<T>(loc.row as usize) })
    }

    /// Referencia mutavel ao componente `T` da entidade.
    ///
    /// Marca o componente como alterado, mesmo que quem chamou nao escreva nada:
    /// o contrato e "foi exposto para escrita" (ver D10).
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        let (cid, loc) = self.localiza_coluna::<T>(entity)?;
        let tick = self.change_tick;
        let arch = self.archetypes.get_mut(loc.archetype)?;
        arch.marcar_alterado(cid, loc.row as usize, tick);
        let coluna = arch.column_mut(cid)?;
        // SAFETY: mesmas condicoes de `get`; a exclusividade vem do `&mut self`.
        Some(unsafe { coluna.get_mut::<T>(loc.row as usize) })
    }

    /// Acrescenta os componentes do bundle a uma entidade existente.
    ///
    /// Componentes que a entidade ja tinha sao substituidos, e os antigos,
    /// destruidos. Devolve `false` se a entidade nao existe.
    ///
    /// # Panics
    ///
    /// Se o bundle declarar o mesmo tipo de componente mais de uma vez.
    pub fn insert<B: Bundle>(&mut self, entity: Entity, bundle: B) -> bool {
        let Some(loc) = self.entities.location(entity) else {
            return false;
        };

        let (mut decl, mut sig) = self.pega_buffers();
        B::register(&mut self.components, &mut decl);
        sig.extend_from_slice(&decl);
        sig.sort_unstable();
        sig.dedup();
        assert_eq!(decl.len(), sig.len(), "bundle com componente repetido");

        let atual = self.archetypes.get(loc.archetype).expect("archetype da entidade");
        // Sobrescrever um componente nao o faz nascer de novo: o tick de
        // insercao de quem ja existia precisa sobreviver a operacao.
        let added_anteriores: Vec<(ComponentId, Tick)> = sig
            .iter()
            .filter_map(|&cid| Some((cid, atual.component_ticks(cid, loc.row as usize)?.added)))
            .collect();

        let mut destino: Vec<ComponentId> = atual.component_ids().to_vec();
        destino.extend_from_slice(&sig);
        let destino = self.archetypes.get_or_insert(&mut destino, &self.components);
        let tick = self.change_tick;

        if destino == loc.archetype {
            // Assinatura inalterada: todos os componentes do bundle ja existiam.
            // Sobrescreve no lugar, sem migrar de archetype.
            let arch = self.archetypes.get_mut(destino).expect("archetype da entidade");
            bundle.take(&mut |i, ptr| {
                let coluna = arch.column_mut(decl[i]).expect("coluna presente na assinatura");
                // SAFETY: `loc.row` e valido e a coluna guarda o tipo de `ptr`,
                // cuja posse `Bundle::take` transfere. O valor antigo e
                // destruido por `replace_at`.
                unsafe { coluna.replace_at(loc.row as usize, ptr) };
            });
            for &cid in &sig {
                arch.marcar_alterado(cid, loc.row as usize, tick);
            }
            self.devolve_buffers(decl, sig);
            return true;
        }

        let (origem, alvo) = self.archetypes.get_pair_mut(loc.archetype, destino);
        // Os componentes do bundle nao sao carregados da origem: seriam
        // sobrescritos logo em seguida. Sao destruidos aqui e reescritos abaixo.
        let (novo_row, deslocada) =
            move_row(origem, loc.row as usize, alvo, entity, &sig, tick, |_| None);

        bundle.take(&mut |i, ptr| {
            let coluna = alvo.column_mut(decl[i]).expect("coluna presente na assinatura");
            // SAFETY: como em `spawn`. `move_row` deixou estas colunas sem o
            // valor da nova linha justamente para que ele seja escrito aqui.
            unsafe { coluna.push_from(ptr) };
        });

        // Componentes que ja existiam mantem seu tick de insercao; so os
        // realmente novos nascem agora.
        if let Some(alvo) = self.archetypes.get_mut(destino) {
            for &(cid, added) in &added_anteriores {
                alvo.restaurar_added(cid, novo_row, added);
            }
        }

        if let Some(d) = deslocada {
            self.entities.set_location(d, loc);
        }
        self.entities
            .set_location(entity, EntityLocation { archetype: destino, row: novo_row as u32 });

        self.devolve_buffers(decl, sig);
        true
    }

    /// Remove o componente `T` da entidade e o devolve.
    ///
    /// Devolve `None` se a entidade nao existe ou nao tem o componente.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> Option<T> {
        let (cid, loc) = self.localiza_coluna::<T>(entity)?;

        let atual = self.archetypes.get(loc.archetype).expect("archetype da entidade");
        let mut destino: Vec<ComponentId> =
            atual.component_ids().iter().copied().filter(|&c| c != cid).collect();
        let destino = self.archetypes.get_or_insert(&mut destino, &self.components);
        debug_assert_ne!(destino, loc.archetype, "remover um componente muda a assinatura");

        let tick = self.change_tick;
        let mut saida = std::mem::MaybeUninit::<T>::uninit();
        let (origem, alvo) = self.archetypes.get_pair_mut(loc.archetype, destino);

        let (novo_row, deslocada) =
            move_row(origem, loc.row as usize, alvo, entity, &[], tick, |c| {
                // O destino nao tem `cid`, entao este e o unico componente que
                // cai no ramo de resgate.
                (c == cid).then(|| saida.as_mut_ptr().cast::<u8>())
            });

        if let Some(d) = deslocada {
            self.entities.set_location(d, loc);
        }
        self.entities
            .set_location(entity, EntityLocation { archetype: destino, row: novo_row as u32 });

        // SAFETY: `localiza_coluna` garantiu que o archetype de origem tinha a
        // coluna de `T` na linha `loc.row`, e o destino nao a tem — entao
        // `move_row` percorreu o ramo de resgate exatamente uma vez e escreveu
        // um `T` valido em `saida`.
        Some(unsafe { saida.assume_init() })
    }

    /// Recursos do mundo.
    #[inline]
    #[must_use]
    pub const fn resources(&self) -> &Resources {
        &self.resources
    }

    /// Insere um recurso, devolvendo o valor anterior se havia um.
    pub fn insert_resource<R: Resource>(&mut self, valor: R) -> Option<R> {
        self.resources.insert(valor)
    }

    /// Remove um recurso e o devolve.
    pub fn remove_resource<R: Resource>(&mut self) -> Option<R> {
        self.resources.remove::<R>()
    }

    /// Indica se o recurso `R` tem valor presente.
    #[must_use]
    pub fn contains_resource<R: Resource>(&self) -> bool {
        self.resources.contains::<R>()
    }

    /// Referencia ao recurso `R`, ou `None` se ele nao existe.
    #[must_use]
    pub fn get_resource<R: Resource>(&self) -> Option<&R> {
        self.resources.get::<R>()
    }

    /// Referencia mutavel ao recurso `R`, ou `None` se ele nao existe.
    pub fn get_resource_mut<R: Resource>(&mut self) -> Option<&mut R> {
        self.resources.get_mut::<R>()
    }

    /// Referencia ao recurso `R`.
    ///
    /// Use quando a ausencia do recurso for erro de programacao, e nao um caso
    /// a tratar — a mensagem do panic nomeia o tipo, o que custa muito menos
    /// tempo de diagnostico do que um `unwrap` anonimo.
    ///
    /// # Panics
    ///
    /// Se o recurso nao tiver sido inserido.
    #[must_use]
    pub fn resource<R: Resource>(&self) -> &R {
        self.resources
            .get::<R>()
            .unwrap_or_else(|| panic!("recurso {} nao foi inserido", std::any::type_name::<R>()))
    }

    /// Referencia mutavel ao recurso `R`.
    ///
    /// # Panics
    ///
    /// Se o recurso nao tiver sido inserido.
    pub fn resource_mut<R: Resource>(&mut self) -> &mut R {
        self.resources
            .get_mut::<R>()
            .unwrap_or_else(|| panic!("recurso {} nao foi inserido", std::any::type_name::<R>()))
    }

    /// Percorre as entidades que tem todos os componentes pedidos por `D`.
    ///
    /// Exige `&mut self` mesmo quando `D` so le: e o emprestimo exclusivo que
    /// sustenta a seguranca da iteracao, descrita em [`crate::query`].
    ///
    /// # Panics
    ///
    /// Se `D` pedir o mesmo componente de forma conflitante, como
    /// `(&mut T, &mut T)` ou `(&T, &mut T)`.
    pub fn query<D: QueryData>(&mut self) -> QueryIter<'_, D, ()> {
        self.query_filtered::<D, ()>()
    }

    /// Como [`query`](Self::query), restrito aos archetypes que passam por `F`.
    ///
    /// # Panics
    ///
    /// Nas mesmas condicoes de [`query`](Self::query).
    pub fn query_filtered<D: QueryData, F: QueryFilter>(&mut self) -> QueryIter<'_, D, F> {
        self.query_filtered_since::<D, F>(Tick::ZERO)
    }

    /// Como [`query_filtered`](Self::query_filtered), com `last_run` explicito.
    ///
    /// Filtros de mudanca comparam contra `last_run`. As versoes sem este
    /// parametro usam [`Tick::ZERO`], o que faz `Changed` e `Added` aceitarem
    /// tudo o que ja existiu — util fora de um sistema, onde nao ha "ultima
    /// execucao" natural.
    pub fn query_filtered_since<D: QueryData, F: QueryFilter>(
        &mut self,
        last_run: Tick,
    ) -> QueryIter<'_, D, F> {
        // Registrar os componentes da query antes de olhar os archetypes faz
        // com que um tipo ainda desconhecido passe a existir no registro. Assim
        // uma query sobre um componente que ninguem usou simplesmente nao casa
        // com nada, em vez de falhar.
        //
        // Os dois emprestimos sao de campos distintos: o mutavel de
        // `components` termina dentro de `QueryIter::new`, e so o compartilhado
        // de `archetypes` sobrevive junto com a query.
        let ticks = SystemTicks::new(last_run, self.change_tick);
        let components = &mut self.components;
        let archetypes = &self.archetypes;
        QueryIter::new(archetypes, components, ticks)
    }

    /// Reserva espaco para `n` entidades no archetype de uma assinatura ja
    /// conhecida, evitando realocacoes durante uma rajada de `spawn`.
    pub fn reserve(&mut self, archetype: ArchetypeId, n: usize) {
        self.entities.reserve(n);
        if let Some(arch) = self.archetypes.get_mut(archetype) {
            arch.reserve(n);
        }
    }

    /// Resolve `T` para `(ComponentId, EntityLocation)` se a entidade existir e
    /// tiver o componente.
    fn localiza_coluna<T: Component>(
        &self,
        entity: Entity,
    ) -> Option<(ComponentId, EntityLocation)> {
        let cid = self.components.id::<T>()?;
        let loc = self.entities.location(entity)?;
        self.archetypes.get(loc.archetype)?.contains(cid).then_some((cid, loc))
    }

    fn pega_buffers(&mut self) -> (Vec<ComponentId>, Vec<ComponentId>) {
        let mut decl = std::mem::take(&mut self.decl);
        let mut sig = std::mem::take(&mut self.sig);
        decl.clear();
        sig.clear();
        (decl, sig)
    }

    fn devolve_buffers(&mut self, decl: Vec<ComponentId>, sig: Vec<ComponentId>) {
        self.decl = decl;
        self.sig = sig;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::tick::Tick;

    #[derive(Debug, PartialEq)]
    struct Posicao(f32, f32);

    #[derive(Debug, PartialEq)]
    struct Velocidade(f32, f32);

    #[derive(Debug, PartialEq)]
    struct Nome(String);

    #[test]
    fn spawn_e_leitura() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 2.0), Velocidade(3.0, 4.0)));

        assert_eq!(w.len(), 1);
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)));
        assert_eq!(w.get::<Velocidade>(e), Some(&Velocidade(3.0, 4.0)));
        assert!(w.has::<Posicao>(e));
    }

    #[test]
    fn componente_ausente_devolve_none() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0, 0.0),));
        assert_eq!(w.get::<Velocidade>(e), None);
        assert!(!w.has::<Velocidade>(e));
    }

    #[test]
    fn mutacao_persiste() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0, 0.0),));

        w.get_mut::<Posicao>(e).unwrap().0 = 42.0;
        assert_eq!(w.get::<Posicao>(e).unwrap().0, 42.0);
    }

    #[test]
    fn despawn_apaga_a_entidade() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));

        assert!(w.despawn(e));
        assert!(!w.contains(e));
        assert_eq!(w.get::<Posicao>(e), None);
        assert!(w.is_empty());
        assert!(!w.despawn(e), "o segundo despawn nao deve ter efeito");
    }

    #[test]
    fn despawn_corrige_a_localizacao_da_entidade_deslocada() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0, 1.0),));
        let b = w.spawn((Posicao(2.0, 2.0),));
        let c = w.spawn((Posicao(3.0, 3.0),));

        // `a` sai da linha 0 e `c` e trazido da ultima linha para o lugar dele.
        w.despawn(a);

        assert_eq!(w.get::<Posicao>(b), Some(&Posicao(2.0, 2.0)));
        assert_eq!(w.get::<Posicao>(c), Some(&Posicao(3.0, 3.0)), "c precisa continuar acessivel");
    }

    #[test]
    fn insert_migra_de_archetype_preservando_o_que_ja_existia() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 2.0),));

        assert!(w.insert(e, (Velocidade(9.0, 9.0),)));

        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)), "posicao deveria ter migrado");
        assert_eq!(w.get::<Velocidade>(e), Some(&Velocidade(9.0, 9.0)));
    }

    #[test]
    fn insert_sobrescreve_componente_existente() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));

        assert!(w.insert(e, (Posicao(7.0, 8.0),)));

        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(7.0, 8.0)));
        assert_eq!(w.archetypes().len(), 2, "a assinatura nao mudou; nenhum archetype novo");
    }

    #[test]
    fn insert_em_entidade_morta_falha() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0, 0.0),));
        w.despawn(e);
        assert!(!w.insert(e, (Velocidade(1.0, 1.0),)));
    }

    #[test]
    fn remove_devolve_o_valor_e_mantem_o_resto() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 2.0), Velocidade(3.0, 4.0)));

        assert_eq!(w.remove::<Velocidade>(e), Some(Velocidade(3.0, 4.0)));
        assert_eq!(w.get::<Velocidade>(e), None);
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)));
    }

    #[test]
    fn remove_de_componente_ausente_devolve_none() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0, 0.0),));
        assert_eq!(w.remove::<Velocidade>(e), None);
    }

    #[test]
    fn componentes_com_drop_sao_destruidos_uma_unica_vez() {
        struct Sentinela(Arc<Mutex<u32>>);
        impl Drop for Sentinela {
            fn drop(&mut self) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let contador = Arc::new(Mutex::new(0));

        let mut w = World::new();
        let e = w.spawn((Sentinela(Arc::clone(&contador)), Posicao(0.0, 0.0)));

        // Migrar de archetype move o sentinela, sem destrui-lo.
        w.insert(e, (Velocidade(1.0, 1.0),));
        assert_eq!(*contador.lock().unwrap(), 0, "migrar nao pode destruir");

        w.despawn(e);
        assert_eq!(*contador.lock().unwrap(), 1, "despawn destroi exatamente uma vez");
    }

    #[test]
    fn remove_transfere_a_posse_de_um_componente_com_drop() {
        let contador = Arc::new(Mutex::new(0));
        struct Sentinela(Arc<Mutex<u32>>);
        impl Drop for Sentinela {
            fn drop(&mut self) {
                *self.0.lock().unwrap() += 1;
            }
        }

        let mut w = World::new();
        let e = w.spawn((Sentinela(Arc::clone(&contador)), Posicao(0.0, 0.0)));

        let resgatado = w.remove::<Sentinela>(e).expect("componente presente");
        assert_eq!(*contador.lock().unwrap(), 0, "o valor foi movido, nao destruido");

        drop(resgatado);
        assert_eq!(*contador.lock().unwrap(), 1);

        // A entidade continua viva, sem o componente.
        assert!(w.contains(e));
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(0.0, 0.0)));
    }

    #[test]
    fn componentes_nao_copy_migram_intactos() {
        let mut w = World::new();
        let e = w.spawn((Nome(String::from("unidade")),));

        w.insert(e, (Posicao(1.0, 1.0),));

        assert_eq!(w.get::<Nome>(e), Some(&Nome(String::from("unidade"))));
    }

    #[test]
    fn muitas_entidades_permanecem_consistentes() {
        let mut w = World::new();
        let entidades: Vec<_> =
            (0..1_000).map(|i| w.spawn((Posicao(i as f32, 0.0), Velocidade(1.0, 0.0)))).collect();

        // Remove metade, em ordem que forca muitos deslocamentos.
        for &e in entidades.iter().step_by(2) {
            w.despawn(e);
        }

        assert_eq!(w.len(), 500);
        for (i, &e) in entidades.iter().enumerate() {
            if i % 2 == 0 {
                assert!(!w.contains(e));
            } else {
                assert_eq!(
                    w.get::<Posicao>(e),
                    Some(&Posicao(i as f32, 0.0)),
                    "entidade {i} perdeu a posicao apos os deslocamentos"
                );
            }
        }
    }

    #[test]
    fn spawn_vazio_usa_o_archetype_vazio() {
        let mut w = World::new();
        let e = w.spawn(());

        assert!(w.contains(e));
        assert_eq!(w.archetypes().len(), 1);
        assert_eq!(w.entities().location(e).unwrap().archetype, ArchetypeId::VAZIO);
    }

    #[test]
    #[should_panic(expected = "componente repetido")]
    fn bundle_com_tipo_repetido_falha_alto() {
        let mut w = World::new();
        w.spawn((Posicao(0.0, 0.0), Posicao(1.0, 1.0)));
    }

    #[derive(Debug, PartialEq)]
    struct Gravidade(f32);

    #[test]
    fn recurso_inserido_e_legivel() {
        let mut w = World::new();
        w.insert_resource(Gravidade(-9.81));

        assert_eq!(w.resource::<Gravidade>(), &Gravidade(-9.81));
        assert!(w.contains_resource::<Gravidade>());
    }

    #[test]
    fn recurso_ausente_devolve_none_em_vez_de_entrar_em_panico() {
        let w = World::new();
        assert_eq!(w.get_resource::<Gravidade>(), None);
        assert!(!w.contains_resource::<Gravidade>());
    }

    #[test]
    #[should_panic(expected = "Gravidade")]
    fn acessar_recurso_ausente_nomeia_o_tipo() {
        let w = World::new();
        let _ = w.resource::<Gravidade>();
    }

    #[test]
    fn recurso_mutavel_persiste() {
        let mut w = World::new();
        w.insert_resource(Gravidade(0.0));
        w.resource_mut::<Gravidade>().0 = -1.62;
        assert_eq!(w.resource::<Gravidade>(), &Gravidade(-1.62));
    }

    #[test]
    fn recurso_e_componente_do_mesmo_tipo_sao_independentes() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 2.0),));
        w.insert_resource(Posicao(100.0, 200.0));

        // Sao dois espacos distintos: mexer em um nao toca no outro.
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)));
        assert_eq!(w.get_resource::<Posicao>(), Some(&Posicao(100.0, 200.0)));

        w.resource_mut::<Posicao>().0 = 0.0;
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(1.0, 2.0)));
    }

    #[test]
    fn recursos_sobrevivem_a_operacoes_com_entidades() {
        let mut w = World::new();
        w.insert_resource(Gravidade(-9.81));

        let e = w.spawn((Posicao(0.0, 0.0),));
        w.insert(e, (Velocidade(1.0, 1.0),));
        w.despawn(e);

        assert_eq!(w.resource::<Gravidade>(), &Gravidade(-9.81));
    }

    #[test]
    fn remover_recurso_devolve_o_valor() {
        let mut w = World::new();
        w.insert_resource(Gravidade(-9.81));

        assert_eq!(w.remove_resource::<Gravidade>(), Some(Gravidade(-9.81)));
        assert!(!w.contains_resource::<Gravidade>());
        assert_eq!(w.remove_resource::<Gravidade>(), None);
    }

    // ---------------------------------------------------- deteccao de mudanca --

    #[test]
    fn componente_recem_criado_conta_como_adicionado() {
        let mut w = World::new();
        w.increment_change_tick();
        let e = w.spawn((Posicao(0.0, 0.0),));

        let t = w.component_ticks::<Posicao>(e).unwrap();
        assert_eq!(t.added, w.change_tick());
        assert_eq!(t.changed, t.added, "inserir tambem conta como alterar");
        assert!(w.is_added::<Posicao>(e, Tick::ZERO));
    }

    #[test]
    fn get_mut_marca_alterado_mesmo_sem_escrita() {
        // O contrato da D10: "foi exposto para escrita", nao "mudou de valor".
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));
        let depois_do_spawn = w.change_tick();

        w.increment_change_tick();
        let _ = w.get_mut::<Posicao>(e);

        assert!(w.is_changed::<Posicao>(e, depois_do_spawn));
    }

    #[test]
    fn leitura_nao_marca_alterado() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));
        let depois_do_spawn = w.change_tick();

        w.increment_change_tick();
        let _ = w.get::<Posicao>(e);

        assert!(!w.is_changed::<Posicao>(e, depois_do_spawn));
    }

    #[test]
    fn migrar_de_archetype_nao_conta_como_alteracao() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));
        let ticks_originais = w.component_ticks::<Posicao>(e).unwrap();

        w.increment_change_tick();
        w.insert(e, (Velocidade(1.0, 1.0),));

        assert_eq!(
            w.component_ticks::<Posicao>(e).unwrap(),
            ticks_originais,
            "o componente foi carregado, nao tocado"
        );
        // O componente novo, esse sim, nasceu agora.
        assert_eq!(w.component_ticks::<Velocidade>(e).unwrap().added, w.change_tick());
    }

    #[test]
    fn sobrescrever_preserva_o_tick_de_insercao() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));
        let nascimento = w.component_ticks::<Posicao>(e).unwrap().added;

        w.increment_change_tick();
        w.insert(e, (Posicao(2.0, 2.0),));

        let t = w.component_ticks::<Posicao>(e).unwrap();
        assert_eq!(t.added, nascimento, "sobrescrever nao faz o componente nascer de novo");
        assert_eq!(t.changed, w.change_tick());
        assert!(!w.is_added::<Posicao>(e, nascimento));
        assert!(w.is_changed::<Posicao>(e, nascimento));
    }

    #[test]
    fn sobrescrever_com_migracao_tambem_preserva_o_added() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0, 1.0),));
        let nascimento = w.component_ticks::<Posicao>(e).unwrap().added;

        w.increment_change_tick();
        // Substitui Posicao e acrescenta Velocidade: muda de archetype.
        w.insert(e, (Posicao(9.0, 9.0), Velocidade(1.0, 1.0)));

        assert_eq!(w.component_ticks::<Posicao>(e).unwrap().added, nascimento);
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(9.0, 9.0)));
    }

    #[test]
    fn ticks_acompanham_a_entidade_deslocada_por_despawn() {
        let mut w = World::new();
        let a = w.spawn((Posicao(1.0, 1.0),));
        w.increment_change_tick();
        let c = w.spawn((Posicao(3.0, 3.0),));
        let ticks_de_c = w.component_ticks::<Posicao>(c).unwrap();

        // `c` e trazido da ultima linha para a linha de `a`.
        w.despawn(a);

        assert_eq!(
            w.component_ticks::<Posicao>(c).unwrap(),
            ticks_de_c,
            "o swap_remove precisa mover os ticks junto com os valores"
        );
    }

    #[test]
    fn saneamento_ajusta_tick_velho_demais() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0, 0.0),));

        // Salta o contador para muito depois do nascimento do componente.
        w.set_change_tick(Tick::new(Tick::IDADE_MAXIMA + 1_000));
        assert_eq!(w.saneia_ticks(), 1, "uma instancia de componente ajustada");

        let t = w.component_ticks::<Posicao>(e).unwrap();
        assert_eq!(t.added.idade(w.change_tick()), Tick::IDADE_MAXIMA);
        assert_eq!(w.saneia_ticks(), 0, "a segunda varredura nao acha nada");
    }

    #[test]
    fn ticks_sobrevivem_a_muitas_operacoes_estruturais() {
        let mut w = World::new();
        let mut ids = Vec::new();
        for i in 0..200 {
            w.increment_change_tick();
            ids.push(w.spawn((Posicao(i as f32, 0.0),)));
        }
        for &e in ids.iter().step_by(3) {
            w.despawn(e);
        }
        for &e in ids.iter().skip(1).step_by(3) {
            w.insert(e, (Velocidade(1.0, 1.0),));
        }

        // Toda entidade viva ainda tem ticks consistentes.
        for &e in &ids {
            if w.contains(e) {
                let t = w.component_ticks::<Posicao>(e).expect("posicao presente");
                assert!(t.added.get() > 0, "tick de insercao perdido");
            }
        }
    }

    #[test]
    fn sequencia_de_operacoes_e_deterministica() {
        let executar = || {
            let mut w = World::new();
            let mut ids = Vec::new();
            for i in 0..50 {
                ids.push(w.spawn((Posicao(i as f32, 0.0),)));
            }
            for &e in ids.iter().step_by(3) {
                w.despawn(e);
            }
            for i in 0..20 {
                ids.push(w.spawn((Posicao(i as f32, 1.0), Velocidade(0.0, 0.0))));
            }
            w.entities().iter().map(|e| (e.index(), e.generation().get())).collect::<Vec<_>>()
        };

        assert_eq!(executar(), executar());
    }
}
