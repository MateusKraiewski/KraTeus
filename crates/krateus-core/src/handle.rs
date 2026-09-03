//! Referencia estavel e tipada para recursos de vida gerenciada.
//!
//! Um [`Handle`] e um par `(indice, geracao)`. O indice permite acesso O(1) em
//! armazenamento contiguo; a geracao invalida handles antigos quando o slot e
//! reciclado. Sem a geracao, liberar um recurso e reutilizar o slot faria um
//! handle antigo passar a apontar, silenciosamente, para o objeto errado — o
//! tipo de bug que so aparece sob carga.
//!
//! O parametro `T` e apenas uma marca: nao existe em tempo de execucao, mas
//! impede que um `Handle<Mesh>` seja usado onde se espera `Handle<Texture>`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;
use std::num::NonZeroU32;

/// Referencia opaca, comparavel e copiavel para um recurso do tipo `T`.
///
/// Ocupa 8 bytes e implementa `Copy` independentemente de `T`.
pub struct Handle<T> {
    index: u32,
    generation: NonZeroU32,
    // `fn() -> T` mantem `Handle<T>` covariante em `T` e, principalmente,
    // `Send`/`Sync` mesmo quando `T` nao e — o handle nao possui o dado.
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    /// Constroi um handle a partir de suas partes.
    ///
    /// Exposto para desserializacao e testes. Em uso normal, handles vem de um
    /// [`Pool`](crate::pool::Pool).
    #[inline]
    #[must_use]
    pub const fn from_raw(index: u32, generation: NonZeroU32) -> Self {
        Self { index, generation, _marker: PhantomData }
    }

    /// Indice do slot referenciado.
    #[inline]
    #[must_use]
    pub const fn index(self) -> u32 {
        self.index
    }

    /// Geracao do slot no momento em que este handle foi emitido.
    #[inline]
    #[must_use]
    pub const fn generation(self) -> NonZeroU32 {
        self.generation
    }
}

// As implementacoes abaixo sao manuais porque `derive` adicionaria um limite
// `T: Clone`, `T: PartialEq` etc. que o handle nao precisa: ele nunca toca em
// um valor de `T`.

impl<T> Clone for Handle<T> {
    #[inline]
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index && self.generation == other.generation
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.index.hash(state);
        self.generation.hash(state);
    }
}

impl<T> fmt::Debug for Handle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Handle<{}>({}v{})", std::any::type_name::<T>(), self.index, self.generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const G1: NonZeroU32 = NonZeroU32::new(1).unwrap();
    const G2: NonZeroU32 = NonZeroU32::new(2).unwrap();

    struct Mesh;

    #[test]
    fn handle_ocupa_oito_bytes() {
        assert_eq!(size_of::<Handle<Mesh>>(), 8);
    }

    #[test]
    fn geracao_diferencia_handles_do_mesmo_slot() {
        let antigo = Handle::<Mesh>::from_raw(7, G1);
        let novo = Handle::<Mesh>::from_raw(7, G2);
        assert_ne!(antigo, novo);
    }

    #[test]
    fn handle_e_send_e_sync_mesmo_com_t_que_nao_e() {
        fn exige<X: Send + Sync>() {}
        exige::<Handle<*const u8>>();
    }
}
