//! Agrupamento de entidades por conjunto de componentes.
//!
//! Um archetype reune todas as entidades que tem exatamente o mesmo conjunto de
//! tipos de componente, guardando uma [`Column`] por tipo. Duas consequencias
//! importam:
//!
//! - Uma query descobre quais entidades casam olhando apenas a assinatura de
//!   cada archetype, e nao entidade por entidade. O custo passa a ser
//!   proporcional ao numero de archetypes, que e pequeno e estavel.
//! - Adicionar ou remover um componente muda a assinatura, e por isso move a
//!   entidade para outro archetype. E a operacao cara do modelo, e a razao de
//!   mudancas estruturais serem diferidas para o fim do passo de simulacao.
//!
//! A assinatura e mantida ordenada e sem duplicatas, de modo que dois conjuntos
//! iguais produzem sempre a mesma chave — e o mesmo archetype — qualquer que
//! tenha sido a ordem em que os componentes foram informados.

use std::collections::HashMap;

use crate::column::Column;
use crate::component::{ComponentId, Components};
use crate::entity::Entity;
use crate::tick::{ComponentTicks, Tick, TickColumn};

/// Identificador denso de um archetype dentro de um [`World`](crate::World).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArchetypeId(u32);

impl ArchetypeId {
    /// Archetype das entidades sem componente nenhum.
    ///
    /// Existe sempre, e e onde toda entidade nasce antes de receber seu
    /// primeiro componente.
    pub const VAZIO: Self = Self(0);

    /// Valor bruto do identificador.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    /// Reconstroi o identificador a partir do indice.
    ///
    /// # Panics
    ///
    /// Se o indice nao couber em `u32`.
    #[inline]
    #[must_use]
    pub const fn from_index(i: usize) -> Self {
        assert!(i <= u32::MAX as usize, "indice de archetype fora do espaco de u32");
        Self(i as u32)
    }
}

/// Conjunto de entidades que compartilham a mesma assinatura de componentes.
#[derive(Debug)]
pub struct Archetype {
    id: ArchetypeId,
    /// Ordenada e sem duplicatas. Paralela a `columns`.
    components: Box<[ComponentId]>,
    columns: Box<[Column]>,
    /// Paralelo a `columns`: um vetor de ticks por coluna, com uma entrada por
    /// linha. Fica fora de `Column` de proposito — ver a D10. Manter aqui deixa
    /// intocado o modulo que concentra o `unsafe`, e evita que quem nao filtra
    /// por mudanca arraste os ticks para o cache.
    ticks: Box<[TickColumn]>,
    entities: Vec<Entity>,
}

impl Archetype {
    fn new(id: ArchetypeId, components: Box<[ComponentId]>, registry: &Components) -> Self {
        debug_assert!(components.windows(2).all(|p| p[0] < p[1]), "assinatura desordenada");

        let columns = components
            .iter()
            .map(|&cid| {
                let info = registry.info(cid).expect("componente da assinatura nao registrado");
                Column::new(info)
            })
            .collect();

        let ticks = (0..components.len()).map(|_| TickColumn::new()).collect();
        Self { id, components, columns, ticks, entities: Vec::new() }
    }

    /// Identificador deste archetype.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> ArchetypeId {
        self.id
    }

    /// Assinatura, ordenada.
    #[inline]
    #[must_use]
    pub fn component_ids(&self) -> &[ComponentId] {
        &self.components
    }

    /// Quantidade de entidades neste archetype.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Indica se o archetype esta vazio.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Entidades armazenadas, na ordem das linhas.
    #[inline]
    #[must_use]
    pub fn entities(&self) -> &[Entity] {
        &self.entities
    }

    /// Entidade da linha `row`.
    #[inline]
    #[must_use]
    pub fn entity_at(&self, row: usize) -> Option<Entity> {
        self.entities.get(row).copied()
    }

    /// Indica se a assinatura contem `id`.
    #[must_use]
    pub fn contains(&self, id: ComponentId) -> bool {
        self.components.binary_search(&id).is_ok()
    }

    /// Posicao da coluna de `id` dentro deste archetype.
    ///
    /// A busca e binaria porque a assinatura e mantida ordenada.
    #[must_use]
    pub fn column_index(&self, id: ComponentId) -> Option<usize> {
        self.components.binary_search(&id).ok()
    }

    pub(crate) fn column(&self, id: ComponentId) -> Option<&Column> {
        self.columns.get(self.column_index(id)?)
    }

    pub(crate) fn column_mut(&mut self, id: ComponentId) -> Option<&mut Column> {
        let i = self.column_index(id)?;
        self.columns.get_mut(i)
    }

    #[cfg(test)]
    pub(crate) fn column_at_mut(&mut self, i: usize) -> &mut Column {
        &mut self.columns[i]
    }

    /// Confere o invariante 2 do [`World`](crate::World): toda coluna tem
    /// exatamente um valor por entidade.
    ///
    /// Roda so em builds de debug. Um archetype que viola isto produz leitura
    /// de memoria nao inicializada mais adiante, e o custo de descobrir isso
    /// perto da causa e muito menor do que longe dela.
    #[inline]
    pub(crate) fn debug_verifica_invariante(&self) {
        #[cfg(debug_assertions)]
        for coluna in &self.columns {
            debug_assert_eq!(
                coluna.len(),
                self.entities.len(),
                "coluna dessincronizada das entidades no archetype {:?}",
                self.id
            );
        }
        #[cfg(debug_assertions)]
        for coluna in &self.ticks {
            debug_assert_eq!(
                coluna.len(),
                self.entities.len(),
                "ticks dessincronizados das entidades no archetype {:?}",
                self.id
            );
        }
    }

    /// Reserva a linha de uma entidade.
    ///
    /// Devolve o indice da linha. As colunas ficam com um valor a menos que as
    /// entidades ate que quem chamou escreva um valor em cada uma — obrigacao
    /// de quem chama, e a razao de este metodo ser interno ao crate.
    pub(crate) fn allocate(&mut self, entity: Entity, tick: Tick) -> usize {
        let row = self.entities.len();
        self.entities.push(entity);
        for coluna in &mut self.ticks {
            coluna.push(ComponentTicks::novo(tick));
        }
        row
    }

    /// Ticks do componente `id` na linha `row`.
    #[must_use]
    pub fn component_ticks(&self, id: ComponentId, row: usize) -> Option<ComponentTicks> {
        let i = self.column_index(id)?;
        self.ticks[i].get(row)
    }

    /// Ponteiro para o inicio da coluna de ticks de indice `i`.
    ///
    /// Usado pelas queries: `&mut T` carimba o tick da linha que entrega, e os
    /// filtros de mudanca leem o tick sem materializar referencia.
    pub(crate) fn ticks_base(
        &self,
        i: usize,
    ) -> std::ptr::NonNull<std::cell::UnsafeCell<ComponentTicks>> {
        self.ticks[i].base()
    }

    /// Marca o componente `id` da linha `row` como exposto para escrita.
    pub(crate) fn marcar_alterado(&mut self, id: ComponentId, row: usize, tick: Tick) {
        if let Some(i) = self.column_index(id) {
            self.ticks[i].set_changed(row, tick);
        }
    }

    /// Preserva o tick de insercao de um componente que ja existia.
    ///
    /// Usado por `World::insert` quando o valor e substituido: sobrescrever nao
    /// faz o componente nascer de novo.
    pub(crate) fn restaurar_added(&mut self, id: ComponentId, row: usize, added: Tick) {
        if let Some(i) = self.column_index(id) {
            self.ticks[i].set_added(row, added);
        }
    }

    /// Aplica a varredura de saneamento a todos os ticks guardados.
    ///
    /// Conta instancias de componente ajustadas, nao ticks individuais.
    pub(crate) fn saneia_ticks(&mut self, agora: Tick) -> usize {
        self.ticks.iter_mut().map(|c| c.saneia(agora)).sum()
    }

    /// Remove a linha `row`, destruindo os componentes dela.
    ///
    /// Devolve a entidade que foi movida de lugar para fechar o buraco, quando
    /// houve movimentacao. Quem chama precisa atualizar a localizacao dessa
    /// entidade.
    pub(crate) fn swap_remove(&mut self, row: usize) -> Option<Entity> {
        for coluna in &mut self.columns {
            coluna.swap_remove_drop(row);
        }
        for coluna in &mut self.ticks {
            coluna.swap_remove(row);
        }
        self.entities.swap_remove(row);
        self.debug_verifica_invariante();
        // `swap_remove` do Vec traz o ultimo para `row`; se `row` era o ultimo,
        // nada se moveu.
        self.entities.get(row).copied()
    }

    /// Reserva espaco para `n` entidades adicionais em todas as colunas.
    pub(crate) fn reserve(&mut self, n: usize) {
        self.entities.reserve(n);
        for coluna in &mut self.columns {
            coluna.reserve(n);
        }
        for coluna in &mut self.ticks {
            coluna.reserve(n);
        }
    }
}

/// Transfere a linha `row` de `origem` para uma linha nova em `destino`.
///
/// Cada componente da origem toma um de tres caminhos:
///
/// - esta em `pular` ou nao existe no destino → sai da origem; se `resgatar`
///   fornecer um ponteiro para ele, e movido para la, senao e destruido;
/// - existe no destino → e movido para a coluna correspondente, sem copia
///   intermediaria pela pilha.
///
/// Devolve a linha ocupada no destino e a entidade que foi deslocada na origem
/// para fechar o buraco, quando houve deslocamento.
pub(crate) fn move_row(
    origem: &mut Archetype,
    row: usize,
    destino: &mut Archetype,
    entity: Entity,
    pular: &[ComponentId],
    tick: Tick,
    mut resgatar: impl FnMut(ComponentId) -> Option<*mut u8>,
) -> (usize, Option<Entity>) {
    debug_assert!(row < origem.entities.len());

    let novo_row = destino.allocate(entity, tick);

    // Percorre por indice: `origem.components[i]` copia o id e nao mantem
    // emprestimo, o que permite pegar `origem.columns[i]` mutavelmente logo em
    // seguida.
    for i in 0..origem.components.len() {
        let cid = origem.components[i];
        let carrega = !pular.contains(&cid) && destino.contains(cid);

        if carrega {
            let j = destino.column_index(cid).expect("contains acabou de confirmar");
            // Migrar de archetype nao e alterar o componente: os ticks vao
            // junto com o valor.
            let carregados = origem.ticks[i].get(row).expect("linha valida na origem");
            destino.ticks[j].set(novo_row, carregados);
            // SAFETY: `push_uninit` devolve um slot nao inicializado com o
            // tamanho e o alinhamento do componente `cid`, e as duas colunas
            // guardam exatamente esse tipo. `swap_remove_move` escreve um valor
            // valido nele, cumprindo o contrato de `push_uninit`. `row` e valido
            // porque o `debug_assert` acima e o invariante de colunas do mesmo
            // comprimento garantem isso.
            unsafe {
                let dst = destino.columns[j].push_uninit();
                origem.columns[i].swap_remove_move(row, dst);
            }
        } else if let Some(dst) = resgatar(cid) {
            // SAFETY: quem forneceu o ponteiro se comprometeu a apontar para
            // espaco valido, alinhado e nao inicializado do tipo de `cid`.
            unsafe { origem.columns[i].swap_remove_move(row, dst) };
        } else {
            origem.columns[i].swap_remove_drop(row);
        }
    }

    for coluna in &mut origem.ticks {
        coluna.swap_remove(row);
    }
    origem.entities.swap_remove(row);

    origem.debug_verifica_invariante();
    // O destino ainda esta com as colunas do bundle por preencher quando
    // `move_row` e usado por `insert`; quem chama verifica ao terminar.

    (novo_row, origem.entities.get(row).copied())
}

/// Todos os archetypes de um `World`, indexados por assinatura.
#[derive(Debug)]
pub struct Archetypes {
    archetypes: Vec<Archetype>,
    por_assinatura: HashMap<Box<[ComponentId]>, ArchetypeId>,
}

impl Archetypes {
    /// Cria a colecao ja com o archetype vazio em [`ArchetypeId::VAZIO`].
    #[must_use]
    pub fn new() -> Self {
        let vazio = Archetype {
            id: ArchetypeId::VAZIO,
            components: Box::new([]),
            columns: Box::new([]),
            ticks: Box::new([]),
            entities: Vec::new(),
        };

        let mut por_assinatura = HashMap::new();
        por_assinatura.insert(Box::<[ComponentId]>::from([]), ArchetypeId::VAZIO);

        Self { archetypes: vec![vazio], por_assinatura }
    }

    /// Quantidade de archetypes existentes.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.archetypes.len()
    }

    /// Sempre `false`: o archetype vazio existe desde a construcao.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// Acesso por identificador.
    #[inline]
    #[must_use]
    pub fn get(&self, id: ArchetypeId) -> Option<&Archetype> {
        self.archetypes.get(id.index())
    }

    /// Acesso mutavel por identificador.
    #[inline]
    pub fn get_mut(&mut self, id: ArchetypeId) -> Option<&mut Archetype> {
        self.archetypes.get_mut(id.index())
    }

    /// Itera sobre os archetypes em ordem de criacao — estavel e portanto
    /// deterministica.
    pub fn iter(&self) -> impl Iterator<Item = &Archetype> {
        self.archetypes.iter()
    }

    /// Devolve o archetype com a assinatura dada, criando-o se necessario.
    ///
    /// `assinatura` e ordenada e desduplicada aqui, de forma que a ordem em que
    /// os componentes chegam nao influencia o resultado.
    pub fn get_or_insert(
        &mut self,
        assinatura: &mut Vec<ComponentId>,
        registry: &Components,
    ) -> ArchetypeId {
        assinatura.sort_unstable();
        assinatura.dedup();

        if let Some(&id) = self.por_assinatura.get(assinatura.as_slice()) {
            return id;
        }

        let id = ArchetypeId(self.archetypes.len() as u32);
        let chave: Box<[ComponentId]> = assinatura.as_slice().into();
        self.archetypes.push(Archetype::new(id, chave.clone(), registry));
        self.por_assinatura.insert(chave, id);
        id
    }

    /// Empresta dois archetypes distintos ao mesmo tempo.
    ///
    /// Necessario para mover uma entidade de um archetype para outro sem clonar
    /// os componentes pelo caminho.
    ///
    /// # Panics
    ///
    /// Se `a` e `b` forem iguais ou se algum nao existir.
    pub(crate) fn get_pair_mut(
        &mut self,
        a: ArchetypeId,
        b: ArchetypeId,
    ) -> (&mut Archetype, &mut Archetype) {
        assert_ne!(a, b, "get_pair_mut exige archetypes distintos");

        let (ia, ib) = (a.index(), b.index());
        if ia < ib {
            let (esq, dir) = self.archetypes.split_at_mut(ib);
            (&mut esq[ia], &mut dir[0])
        } else {
            let (esq, dir) = self.archetypes.split_at_mut(ia);
            (&mut dir[0], &mut esq[ib])
        }
    }
}

impl Default for Archetypes {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comeca_com_o_archetype_vazio() {
        let a = Archetypes::new();
        assert_eq!(a.len(), 1);
        let vazio = a.get(ArchetypeId::VAZIO).unwrap();
        assert!(vazio.component_ids().is_empty());
        assert!(vazio.is_empty());
    }

    #[test]
    fn assinaturas_iguais_em_ordens_diferentes_dao_o_mesmo_archetype() {
        let mut registry = Components::new();
        let pos = registry.register::<u32>();
        let vel = registry.register::<u64>();

        let mut a = Archetypes::new();
        let x = a.get_or_insert(&mut vec![pos, vel], &registry);
        let y = a.get_or_insert(&mut vec![vel, pos], &registry);

        assert_eq!(x, y);
        assert_eq!(a.len(), 2, "so o vazio e um novo deveriam existir");
    }

    #[test]
    fn assinatura_com_repeticao_e_desduplicada() {
        let mut registry = Components::new();
        let pos = registry.register::<u32>();

        let mut a = Archetypes::new();
        let id = a.get_or_insert(&mut vec![pos, pos, pos], &registry);

        assert_eq!(a.get(id).unwrap().component_ids(), &[pos]);
    }

    #[test]
    fn assinaturas_distintas_dao_archetypes_distintos() {
        let mut registry = Components::new();
        let pos = registry.register::<u32>();
        let vel = registry.register::<u64>();

        let mut a = Archetypes::new();
        let so_pos = a.get_or_insert(&mut vec![pos], &registry);
        let ambos = a.get_or_insert(&mut vec![pos, vel], &registry);

        assert_ne!(so_pos, ambos);
        assert_eq!(a.len(), 3);
    }

    #[test]
    fn contains_e_column_index_seguem_a_assinatura() {
        let mut registry = Components::new();
        let pos = registry.register::<u32>();
        let vel = registry.register::<u64>();
        let tag = registry.register::<i8>();

        let mut a = Archetypes::new();
        let id = a.get_or_insert(&mut vec![vel, pos], &registry);
        let arch = a.get(id).unwrap();

        assert!(arch.contains(pos));
        assert!(arch.contains(vel));
        assert!(!arch.contains(tag));

        // A assinatura foi ordenada, entao `pos` (id menor) vem antes.
        assert_eq!(arch.column_index(pos), Some(0));
        assert_eq!(arch.column_index(vel), Some(1));
        assert_eq!(arch.column_index(tag), None);
    }

    #[test]
    fn swap_remove_devolve_a_entidade_deslocada() {
        use std::num::NonZeroU32;

        let mut registry = Components::new();
        let pos = registry.register::<u32>();
        let mut a = Archetypes::new();
        let id = a.get_or_insert(&mut vec![pos], &registry);
        let arch = a.get_mut(id).unwrap();

        let g = NonZeroU32::new(1).unwrap();
        let entidades: Vec<_> = (0..3).map(|i| Entity::from_raw(i, g)).collect();
        for &e in &entidades {
            let row = arch.allocate(e, Tick::ZERO);
            arch.column_at_mut(0).push(row as u32);
        }

        // Remover o meio traz o ultimo para o lugar dele.
        assert_eq!(arch.swap_remove(1), Some(entidades[2]));
        assert_eq!(arch.entities(), &[entidades[0], entidades[2]]);

        // Remover o ultimo nao desloca ninguem.
        assert_eq!(arch.swap_remove(1), None);
        assert_eq!(arch.len(), 1);
    }

    #[test]
    fn get_pair_mut_funciona_nas_duas_ordens() {
        let mut registry = Components::new();
        let pos = registry.register::<u32>();
        let vel = registry.register::<u64>();

        let mut a = Archetypes::new();
        let x = a.get_or_insert(&mut vec![pos], &registry);
        let y = a.get_or_insert(&mut vec![pos, vel], &registry);

        let (p, q) = a.get_pair_mut(x, y);
        assert_eq!((p.id(), q.id()), (x, y));

        let (p, q) = a.get_pair_mut(y, x);
        assert_eq!((p.id(), q.id()), (y, x));
    }
}
