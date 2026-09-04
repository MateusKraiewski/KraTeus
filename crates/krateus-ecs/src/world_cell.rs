//! Acesso ao [`World`] compartilhavel entre threads.
//!
//! `&mut World` e exclusivo por definicao: dois sistemas nao podem te-lo ao
//! mesmo tempo, ainda que toquem dados completamente disjuntos. Isso e correto
//! como regra geral e inutil como regra para um scheduler, cuja razao de existir
//! e justamente rodar junto o que nao se sobrepoe.
//!
//! [`WorldCell`] troca a garantia do compilador por uma garantia verificada:
//! quem o usa precisa ter provado a disjuncao antes. No scheduler, essa prova e
//! o [`Access`](crate::Access), derivado dos tipos de cada sistema.
//!
//! # O que sustenta a seguranca
//!
//! Durante a execucao paralela de uma subetapa, **nenhum sistema materializa
//! `&mut World`**. Cada um deriva apenas o que declarou:
//!
//! - `Query` pega `&Archetypes` — compartilhado, e varios podem te-lo. As
//!   referencias mutaveis a componentes vem de `Column::base`, cujo ponteiro
//!   carrega a proveniencia da alocacao e nao a da referencia compartilhada.
//! - `ResMut` passa por `Resources::get_ptr`, que usa `UnsafeCell` para obter um
//!   ponteiro para um slot especifico. Slots de recursos distintos sao regioes
//!   independentes.
//!
//! [`world_mut`](WorldCell::world_mut) existe para os trechos **sequenciais** —
//! inicializacao dos sistemas e aplicacao dos command buffers — onde o
//! scheduler detem o mundo sozinho.

use std::marker::PhantomData;
use std::ptr::NonNull;

use crate::world::World;

/// Ponteiro para um [`World`] emprestado mutavelmente, compartilhavel entre
/// threads.
///
/// `Copy`, porque cada sistema de uma subetapa recebe o mesmo acesso.
pub struct WorldCell<'w> {
    ptr: NonNull<World>,
    /// Amarra a celula ao emprestimo de onde ela veio, e a torna invariante em
    /// `'w`: nada pode alarga-lo nem encurta-lo por coercao.
    _marker: PhantomData<&'w mut World>,
}

impl<'w> WorldCell<'w> {
    /// Cria uma celula a partir de um emprestimo exclusivo.
    ///
    /// Enquanto a celula existir, o `World` nao deve ser alcancado por nenhum
    /// outro caminho — o emprestimo original fica retido por `'w`.
    #[must_use]
    pub fn new(world: &'w mut World) -> Self {
        Self { ptr: NonNull::from(world), _marker: PhantomData }
    }

    /// Acesso compartilhado ao mundo.
    ///
    /// E por aqui que os parametros de sistema chegam aos archetypes e aos
    /// recursos durante a execucao paralela.
    ///
    /// # Safety
    ///
    /// Nenhum `&mut World` derivado desta celula pode estar vivo. Como varias
    /// referencias compartilhadas coexistem sem problema, chamar isto em
    /// paralelo e legitimo — o que nao pode e misturar com
    /// [`world_mut`](Self::world_mut).
    #[inline]
    #[must_use]
    pub unsafe fn world(self) -> &'w World {
        // SAFETY: o ponteiro veio de uma referencia valida em `new` e o `'w` do
        // retorno e o mesmo daquele emprestimo, que segue retido.
        unsafe { self.ptr.as_ref() }
    }

    /// Acesso exclusivo ao mundo.
    ///
    /// Destinado aos trechos sequenciais do scheduler: inicializar sistemas,
    /// aplicar command buffers, avancar o passo.
    ///
    /// # Safety
    ///
    /// Nenhuma outra referencia derivada desta celula — compartilhada ou
    /// exclusiva — pode estar viva. Na pratica: so chamar quando nenhuma tarefa
    /// paralela estiver em voo.
    #[inline]
    #[must_use]
    pub unsafe fn world_mut(self) -> &'w mut World {
        // SAFETY: como em `world`, com a exclusividade transferida pelo
        // contrato para quem chama. Passa pelo ponteiro cru em vez de
        // `NonNull::as_mut`, que exigiria emprestar `self` mutavelmente — e a
        // celula e `Copy`, recebida por valor.
        unsafe { &mut *self.ptr.as_ptr() }
    }
}

impl Clone for WorldCell<'_> {
    fn clone(&self) -> Self {
        *self
    }
}

impl Copy for WorldCell<'_> {}

// SAFETY: a celula e um ponteiro, sem posse. Enviar e compartilhar o ponteiro
// entre threads so vira problema quando alguem o desreferencia, e as duas
// funcoes que fazem isso sao `unsafe` e exigem que a disjuncao ja tenha sido
// provada. `World` e composto de tipos `Send + Sync` — `Component` e `Resource`
// exigem os dois —, entao o dado alcancavel daqui pode cruzar threads.
unsafe impl Send for WorldCell<'_> {}
// SAFETY: mesmo argumento de `Send`.
unsafe impl Sync for WorldCell<'_> {}

impl std::fmt::Debug for WorldCell<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorldCell").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Posicao(f32);

    #[test]
    fn celula_alcanca_o_mundo() {
        let mut world = World::new();
        let e = world.spawn((Posicao(1.0),));

        let cell = WorldCell::new(&mut world);
        // SAFETY: nenhuma outra referencia derivada da celula esta viva.
        let w = unsafe { cell.world() };
        assert!(w.contains(e));
    }

    #[test]
    fn acesso_exclusivo_permite_mutacao() {
        let mut world = World::new();
        let e = world.spawn((Posicao(1.0),));

        let cell = WorldCell::new(&mut world);
        // SAFETY: nada mais derivado da celula esta vivo.
        unsafe { cell.world_mut() }.get_mut::<Posicao>(e).unwrap().0 = 9.0;

        assert_eq!(world.get::<Posicao>(e).map(|p| p.0), Some(9.0));
    }

    #[test]
    fn a_celula_e_send_e_sync() {
        fn exige<T: Send + Sync>() {}
        exige::<WorldCell<'_>>();
    }

    #[test]
    fn copias_da_celula_alcancam_o_mesmo_mundo() {
        let mut world = World::new();
        world.spawn((Posicao(1.0),));

        let a = WorldCell::new(&mut world);
        let b = a;

        // SAFETY: duas referencias compartilhadas coexistem sem problema.
        unsafe {
            assert_eq!(a.world().len(), b.world().len());
        }
    }
}
