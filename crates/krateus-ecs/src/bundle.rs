//! Conjuntos de componentes entregues de uma vez.
//!
//! `world.spawn((Posicao(..), Velocidade(..)))` cria a entidade e escreve os
//! dois componentes em uma unica passagem, sem mudar de archetype no meio do
//! caminho. Sem isso, cada componente adicional forcaria uma migracao — a
//! operacao mais cara do modelo.
//!
//! Apenas tuplas implementam [`Bundle`]. Um componente solto e escrito como
//! tupla de um elemento: `(Posicao(..),)`. A alternativa — um `impl` generico
//! para qualquer `T: Component` — colidiria com o `impl` das tuplas, ja que
//! tuplas tambem sao componentes validos.

use std::mem::ManuallyDrop;

use crate::component::{Component, ComponentId, Components};

/// Conjunto de componentes que pode ser inserido de uma vez.
///
/// A implementacao e gerada para tuplas de ate 12 elementos; nao ha nada para
/// escrever a mao.
pub trait Bundle: Send + Sync + 'static {
    /// Registra os tipos do conjunto e acrescenta os identificadores a `out`,
    /// na ordem de declaracao.
    fn register(components: &mut Components, out: &mut Vec<ComponentId>);

    /// Entrega cada componente ao escritor, por posicao de declaracao.
    ///
    /// O ponteiro recebido aponta para um valor valido cuja posse passa para o
    /// escritor: ele precisa move-lo para algum lugar, senao o valor vaza.
    /// As posicoes correspondem, uma a uma, aos identificadores acrescentados
    /// por [`register`](Bundle::register).
    fn take(self, escritor: &mut dyn FnMut(usize, *mut u8));
}

impl Bundle for () {
    fn register(_: &mut Components, _: &mut Vec<ComponentId>) {}
    fn take(self, _: &mut dyn FnMut(usize, *mut u8)) {}
}

macro_rules! impl_bundle {
    ($(($T:ident, $i:tt)),+) => {
        impl<$($T: Component),+> Bundle for ($($T,)+) {
            fn register(components: &mut Components, out: &mut Vec<ComponentId>) {
                $(out.push(components.register::<$T>());)+
            }

            #[allow(non_snake_case)]
            fn take(self, escritor: &mut dyn FnMut(usize, *mut u8)) {
                let ($($T,)+) = self;
                $(
                    // `ManuallyDrop` transfere a responsabilidade de destruir
                    // para quem receber o ponteiro.
                    let mut $T = ManuallyDrop::new($T);
                    escritor($i, (&raw mut $T).cast::<u8>());
                )+
            }
        }
    };
}

impl_bundle!((A, 0));
impl_bundle!((A, 0), (B, 1));
impl_bundle!((A, 0), (B, 1), (C, 2));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6), (H, 7));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6), (H, 7), (I, 8));
impl_bundle!((A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6), (H, 7), (I, 8), (J, 9));
impl_bundle!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10)
);
impl_bundle!(
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7),
    (I, 8),
    (J, 9),
    (K, 10),
    (L, 11)
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registra_na_ordem_de_declaracao() {
        let mut c = Components::new();
        let mut ids = Vec::new();
        <(u8, u64, u16) as Bundle>::register(&mut c, &mut ids);

        assert_eq!(ids.len(), 3);
        assert_eq!(ids[0], c.id::<u8>().unwrap());
        assert_eq!(ids[1], c.id::<u64>().unwrap());
        assert_eq!(ids[2], c.id::<u16>().unwrap());
    }

    #[test]
    fn take_entrega_cada_posicao_uma_vez() {
        let mut vistos = Vec::new();
        (1_u8, 2_u64, 3_u16).take(&mut |i, ptr| {
            // SAFETY: o ponteiro aponta para o componente daquela posicao, com
            // o tipo correspondente. A posse e assumida aqui.
            let valor = unsafe {
                match i {
                    0 => u64::from(ptr.cast::<u8>().read()),
                    1 => ptr.cast::<u64>().read(),
                    2 => u64::from(ptr.cast::<u16>().read()),
                    _ => unreachable!("posicao fora do bundle"),
                }
            };
            vistos.push((i, valor));
        });

        assert_eq!(vistos, vec![(0, 1), (1, 2), (2, 3)]);
    }

    #[test]
    fn bundle_vazio_nao_registra_nada() {
        let mut c = Components::new();
        let mut ids = Vec::new();
        <() as Bundle>::register(&mut c, &mut ids);
        assert!(ids.is_empty());
        assert!(c.is_empty());
    }
}
