//! Armazenamento denso com slots reciclados e acesso por [`Handle`].
//!
//! O `Pool` e o primeiro primitivo do secao 11 do documento de visao: os slots
//! sao reutilizados em vez de liberados, de modo que um ciclo de
//! criacao/destruicao em regime estavel para de alocar. A geracao do handle e
//! quem torna a reciclagem segura.

use std::num::NonZeroU32;

use crate::handle::Handle;

const PRIMEIRA_GERACAO: NonZeroU32 = NonZeroU32::new(1).unwrap();

/// Colecao de `T` enderecada por [`Handle<T>`].
///
/// Inserir e remover sao O(1). Handles emitidos antes de uma remocao param de
/// resolver, sem nunca resolver para o objeto errado.
#[derive(Debug)]
pub struct Pool<T> {
    slots: Vec<Option<T>>,
    generations: Vec<NonZeroU32>,
    livres: Vec<u32>,
    ocupados: usize,
}

impl<T> Pool<T> {
    /// Cria um pool vazio, sem alocar.
    #[must_use]
    pub const fn new() -> Self {
        Self { slots: Vec::new(), generations: Vec::new(), livres: Vec::new(), ocupados: 0 }
    }

    /// Cria um pool com capacidade reservada para `n` elementos.
    ///
    /// Preferir isto quando a ordem de grandeza e conhecida: evita a cascata de
    /// realocacoes durante o aquecimento.
    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        Self {
            slots: Vec::with_capacity(n),
            generations: Vec::with_capacity(n),
            livres: Vec::new(),
            ocupados: 0,
        }
    }

    /// Quantidade de elementos vivos.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.ocupados
    }

    /// Indica se nao ha elementos vivos.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.ocupados == 0
    }

    /// Numero de slots ja alocados, vivos ou reciclaveis.
    #[must_use]
    pub fn capacity_used(&self) -> usize {
        self.slots.len()
    }

    /// Insere `valor` e devolve o handle correspondente.
    pub fn insert(&mut self, valor: T) -> Handle<T> {
        self.ocupados += 1;

        if let Some(index) = self.livres.pop() {
            let i = index as usize;
            self.slots[i] = Some(valor);
            return Handle::from_raw(index, self.generations[i]);
        }

        let index = u32::try_from(self.slots.len()).expect("Pool excedeu u32::MAX slots");
        self.slots.push(Some(valor));
        self.generations.push(PRIMEIRA_GERACAO);
        Handle::from_raw(index, PRIMEIRA_GERACAO)
    }

    /// Indica se `handle` ainda resolve para um elemento vivo.
    #[must_use]
    pub fn contains(&self, handle: Handle<T>) -> bool {
        self.resolve(handle).is_some()
    }

    /// Acesso imutavel, ou `None` se o handle expirou.
    #[must_use]
    pub fn get(&self, handle: Handle<T>) -> Option<&T> {
        self.slots[self.resolve(handle)?].as_ref()
    }

    /// Acesso mutavel, ou `None` se o handle expirou.
    #[must_use]
    pub fn get_mut(&mut self, handle: Handle<T>) -> Option<&mut T> {
        let i = self.resolve(handle)?;
        self.slots[i].as_mut()
    }

    /// Remove o elemento e devolve seu valor, invalidando todos os handles que
    /// apontavam para ele.
    ///
    /// O slot volta para a lista de livres. Quando a geracao satura, o slot e
    /// aposentado em vez de reciclado: e mais barato vazar um slot do que
    /// arriscar dois handles distintos colidindo.
    pub fn remove(&mut self, handle: Handle<T>) -> Option<T> {
        let i = self.resolve(handle)?;
        let valor = self.slots[i].take()?;
        self.ocupados -= 1;

        match self.generations[i].checked_add(1) {
            Some(proxima) => {
                self.generations[i] = proxima;
                self.livres.push(handle.index());
            }
            None => tracing::warn!(slot = i, "geracao saturada; slot aposentado"),
        }

        Some(valor)
    }

    /// Remove todos os elementos, preservando a memoria ja alocada.
    pub fn clear(&mut self) {
        for i in 0..self.slots.len() {
            if self.slots[i].take().is_none() {
                continue;
            }
            if let Some(proxima) = self.generations[i].checked_add(1) {
                self.generations[i] = proxima;
                self.livres.push(i as u32);
            }
        }
        self.ocupados = 0;
    }

    /// Itera sobre os elementos vivos, com seus handles.
    ///
    /// A ordem e a ordem de slot, estavel entre execucoes — requisito dos
    /// testes de determinismo (secao 15 do documento de visao).
    pub fn iter(&self) -> impl Iterator<Item = (Handle<T>, &T)> {
        self.slots.iter().enumerate().filter_map(|(i, slot)| {
            let valor = slot.as_ref()?;
            Some((Handle::from_raw(i as u32, self.generations[i]), valor))
        })
    }

    /// Itera mutavelmente sobre os elementos vivos.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Handle<T>, &mut T)> {
        let generations = &self.generations;
        self.slots.iter_mut().enumerate().filter_map(move |(i, slot)| {
            let valor = slot.as_mut()?;
            Some((Handle::from_raw(i as u32, generations[i]), valor))
        })
    }

    /// Converte um handle no indice do slot, se ele ainda for valido.
    fn resolve(&self, handle: Handle<T>) -> Option<usize> {
        let i = handle.index() as usize;
        (self.generations.get(i) == Some(&handle.generation())).then_some(i)
    }
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insere_e_le() {
        let mut pool = Pool::new();
        let h = pool.insert("mesh");
        assert_eq!(pool.get(h), Some(&"mesh"));
        assert_eq!(pool.len(), 1);
    }

    #[test]
    fn handle_removido_para_de_resolver() {
        let mut pool = Pool::new();
        let h = pool.insert(42);
        assert_eq!(pool.remove(h), Some(42));
        assert_eq!(pool.get(h), None);
        assert!(pool.is_empty());
    }

    #[test]
    fn slot_reciclado_nao_resolve_handle_antigo() {
        let mut pool = Pool::new();
        let antigo = pool.insert(1);
        pool.remove(antigo);
        let novo = pool.insert(2);

        // O slot e o mesmo, mas os handles sao distintos e o antigo esta morto.
        assert_eq!(antigo.index(), novo.index());
        assert_eq!(pool.get(antigo), None);
        assert_eq!(pool.get(novo), Some(&2));
    }

    #[test]
    fn ciclo_estavel_nao_cresce_o_armazenamento() {
        let mut pool = Pool::with_capacity(8);
        for _ in 0..1_000 {
            let h = pool.insert(0_u64);
            pool.remove(h);
        }
        assert_eq!(pool.capacity_used(), 1, "o slot deveria ser reutilizado sempre");
    }

    #[test]
    fn iteracao_segue_ordem_de_slot() {
        let mut pool = Pool::new();
        let a = pool.insert('a');
        let b = pool.insert('b');
        let c = pool.insert('c');
        pool.remove(b);

        let vistos: Vec<_> = pool.iter().map(|(h, v)| (h, *v)).collect();
        assert_eq!(vistos, vec![(a, 'a'), (c, 'c')]);
    }

    #[test]
    fn clear_invalida_todos_os_handles() {
        let mut pool = Pool::new();
        let h = pool.insert(9);
        pool.clear();
        assert!(pool.is_empty());
        assert!(!pool.contains(h));
    }
}
