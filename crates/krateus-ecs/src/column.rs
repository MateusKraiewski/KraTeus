//! Vetor type-erased de valores de um unico tipo de componente.
//!
//! Um archetype guarda seus componentes em colunas paralelas: todos os valores
//! de `Posicao` juntos, todos os de `Velocidade` juntos. E esse layout — SoA —
//! que faz um sistema que le so `Posicao` percorrer memoria contigua em vez de
//! saltar sobre campos que nao usa, e e dele que vem o ganho de cache descrito
//! na secao 11 do documento de visao.
//!
//! Como o archetype nao conhece os tipos que carrega, a coluna manipula bytes
//! crus guiada pelo [`ComponentInfo`]. Todo o `unsafe` do armazenamento esta
//! concentrado aqui, de proposito: e a unica forma de manter o resto do ECS
//! auditavel.
//!
//! Invariante central mantida por este modulo: as posicoes `0..len` sempre
//! contem valores inicializados do tipo descrito por `info`, e `len <= cap`.

use std::alloc::Layout;
#[cfg(test)]
use std::mem::ManuallyDrop;
use std::ptr::NonNull;

use crate::component::{Component, ComponentInfo};

pub(crate) struct Column {
    /// Aponta para `cap` slots. Quando `cap == 0` ou o tipo e ZST, e um
    /// ponteiro pendurado porem alinhado, nunca desreferenciado para leitura
    /// de bytes.
    data: NonNull<u8>,
    len: usize,
    cap: usize,
    item_layout: Layout,
    drop: Option<unsafe fn(*mut u8)>,
}

// SAFETY: uma coluna so recebe valores de tipos que implementam `Component`, e
// esse trait exige `Send + Sync`. O ponteiro cru nao adiciona compartilhamento
// nenhum alem do que os proprios valores ja permitem; ele existe apenas porque
// o tipo foi apagado. Sem estas implementacoes, `NonNull<u8>` tornaria todo o
// `World` !Send, inviabilizando o scheduler paralelo.
unsafe impl Send for Column {}
// SAFETY: mesmo argumento de `Send`.
unsafe impl Sync for Column {}

impl Column {
    pub(crate) fn new(info: &ComponentInfo) -> Self {
        let item_layout = info.layout();
        Self {
            data: dangling_alinhado(item_layout),
            len: 0,
            // Um ZST nao ocupa espaco, entao nunca precisa crescer. Fingir
            // capacidade infinita elimina o caso especial de todo o resto.
            cap: if item_layout.size() == 0 { usize::MAX } else { 0 },
            item_layout,
            drop: info.drop_fn(),
        }
    }

    #[inline]
    pub(crate) const fn len(&self) -> usize {
        self.len
    }

    /// Auxiliares exercitados apenas pelos testes deste modulo. Ficam sob
    /// `cfg(test)` em vez de `allow(dead_code)` para que o compilador continue
    /// avisando se algum codigo de producao deixar de ser usado.
    #[cfg(test)]
    pub(crate) const fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[cfg(test)]
    pub(crate) const fn capacity(&self) -> usize {
        self.cap
    }

    /// Ponteiro para o inicio da coluna.
    ///
    /// O ponteiro devolvido carrega a proveniencia da alocacao, e nao a do
    /// `&self` usado para le-lo: escrever atraves dele e legitimo mesmo tendo
    /// partido de uma referencia compartilhada. E isso que permite a uma query
    /// segurar `&Archetypes` e ainda assim entregar `&mut T`, desde que a
    /// exclusividade seja garantida por outro meio — no caso, por a query so
    /// poder nascer de um `&mut World`.
    #[inline]
    pub(crate) const fn base(&self) -> NonNull<u8> {
        self.data
    }

    /// Ponteiro para o slot `row`. Nao valida nada.
    ///
    /// # Safety
    ///
    /// `row` precisa ser menor que `cap`.
    #[inline]
    pub(crate) unsafe fn ptr_at(&self, row: usize) -> *mut u8 {
        // SAFETY: o contrato exige `row < cap`, e a alocacao cobre `cap`
        // slots de `item_layout.size()` bytes, entao o deslocamento cai dentro
        // do mesmo objeto alocado. Para ZST o tamanho e zero e o ponteiro
        // resultante e o proprio ponteiro base, alinhado.
        unsafe { self.data.as_ptr().add(row * self.item_layout.size()) }
    }

    /// Garante espaco para mais `additional` valores.
    pub(crate) fn reserve(&mut self, additional: usize) {
        if self.item_layout.size() == 0 {
            return;
        }
        let necessaria = self.len.checked_add(additional).expect("capacidade de coluna estourou");
        if necessaria <= self.cap {
            return;
        }
        // Crescimento geometrico: amortiza o custo de realocar ao longo das
        // insercoes, que e o que a secao 11 pede ao falar em evitar alocacao
        // por frame.
        let nova = necessaria.max(self.cap.saturating_mul(2)).max(4);
        self.realoca(nova);
    }

    fn realoca(&mut self, nova_cap: usize) {
        debug_assert!(nova_cap > self.cap);
        debug_assert!(self.item_layout.size() > 0);

        let novo_layout = layout_de_array(self.item_layout, nova_cap);
        let novo = if self.cap == 0 {
            // SAFETY: `novo_layout` tem tamanho maior que zero, ja que
            // `item_layout.size() > 0` e `nova_cap > 0`.
            unsafe { std::alloc::alloc(novo_layout) }
        } else {
            let antigo = layout_de_array(self.item_layout, self.cap);
            // SAFETY: `self.data` veio de `alloc`/`realloc` com o layout
            // `antigo`, o alinhamento e o mesmo nos dois layouts, e
            // `novo_layout.size()` e maior que zero.
            unsafe { std::alloc::realloc(self.data.as_ptr(), antigo, novo_layout.size()) }
        };

        self.data =
            NonNull::new(novo).unwrap_or_else(|| std::alloc::handle_alloc_error(novo_layout));
        self.cap = nova_cap;
    }

    /// Move para o fim da coluna o valor apontado por `src`.
    ///
    /// # Safety
    ///
    /// `src` precisa apontar para um valor valido e alinhado do tipo desta
    /// coluna. A posse do valor passa para a coluna: quem chama nao pode mais
    /// usa-lo nem deixar que seja destruido.
    pub(crate) unsafe fn push_from(&mut self, src: *const u8) {
        self.reserve(1);
        let size = self.item_layout.size();
        if size > 0 {
            // SAFETY: `reserve(1)` garantiu `len < cap`, entao `ptr_at(len)`
            // aponta para um slot valido e nao inicializado. Origem e destino
            // sao alocacoes distintas, logo nao se sobrepoem.
            unsafe { std::ptr::copy_nonoverlapping(src, self.ptr_at(self.len), size) };
        }
        self.len += 1;
    }

    /// Reserva um slot no fim e devolve o ponteiro para ele, sem inicializar.
    ///
    /// E o destino de uma transferencia vinda de outro archetype: evita passar
    /// o componente pela pilha so para reempurra-lo.
    ///
    /// # Safety
    ///
    /// Quem chama precisa escrever um valor valido do tipo desta coluna no
    /// ponteiro devolvido antes de qualquer outra operacao sobre ela. Ate la, a
    /// coluna esta com um slot logicamente vivo e fisicamente nao inicializado.
    pub(crate) unsafe fn push_uninit(&mut self) -> *mut u8 {
        self.reserve(1);
        // SAFETY: `reserve(1)` garantiu `len < cap`.
        let ptr = unsafe { self.ptr_at(self.len) };
        self.len += 1;
        ptr
    }

    /// Substitui o valor em `row`, destruindo o antigo.
    ///
    /// # Safety
    ///
    /// `row < len` e `src` precisa apontar para um valor valido do tipo desta
    /// coluna, cuja posse passa para ca.
    pub(crate) unsafe fn replace_at(&mut self, row: usize, src: *const u8) {
        debug_assert!(row < self.len);

        if let Some(drop) = self.drop {
            // SAFETY: `row < len` pelo contrato, entao o slot contem um valor
            // inicializado; ele e sobrescrito logo abaixo.
            unsafe { drop(self.ptr_at(row)) };
        }

        let size = self.item_layout.size();
        if size > 0 {
            // SAFETY: destino valido pelo contrato; regioes distintas.
            unsafe { std::ptr::copy_nonoverlapping(src, self.ptr_at(row), size) };
        }
    }

    /// Empurra um valor tipado, consumindo-o.
    ///
    /// O `World` usa `push_from`, que evita passar o valor pela pilha; esta
    /// versao tipada existe para os testes.
    #[cfg(test)]
    pub(crate) fn push<T: Component>(&mut self, valor: T) {
        debug_assert_eq!(self.item_layout, Layout::new::<T>());

        // `ManuallyDrop` impede que `valor` seja destruido ao sair de escopo:
        // a posse esta indo para a coluna.
        let mut valor = ManuallyDrop::new(valor);
        let src = (&raw mut valor).cast::<u8>();
        // SAFETY: `src` aponta para um `T` valido e alinhado, e `ManuallyDrop`
        // tem o mesmo layout de `T`. O `debug_assert` acima confirma que este
        // e o tipo da coluna.
        unsafe { self.push_from(src) };
    }

    /// Remove o valor em `row`, destruindo-o, e traz o ultimo para o lugar.
    ///
    /// Devolve a linha que foi movida, quando houve movimentacao — o archetype
    /// usa isso para corrigir a localizacao da entidade deslocada.
    pub(crate) fn swap_remove_drop(&mut self, row: usize) -> Option<usize> {
        assert!(row < self.len, "linha {row} fora da coluna de tamanho {}", self.len);

        if let Some(drop) = self.drop {
            // SAFETY: `row < len`, entao o slot contem um valor inicializado do
            // tipo ao qual `drop` pertence. Depois desta chamada o slot e
            // sobrescrito ou a coluna encolhe, entao ninguem o le de novo.
            unsafe { drop(self.ptr_at(row)) };
        }
        self.fecha_buraco(row)
    }

    /// Move o valor em `row` para `dst` sem destrui-lo, e traz o ultimo para o
    /// lugar.
    ///
    /// E o que permite transferir um componente de um archetype para outro
    /// quando a entidade ganha ou perde um componente.
    ///
    /// # Safety
    ///
    /// `dst` precisa apontar para espaco valido, alinhado e nao inicializado,
    /// do tamanho do tipo desta coluna. A posse do valor passa para `dst`.
    pub(crate) unsafe fn swap_remove_move(&mut self, row: usize, dst: *mut u8) -> Option<usize> {
        assert!(row < self.len, "linha {row} fora da coluna de tamanho {}", self.len);

        let size = self.item_layout.size();
        if size > 0 {
            // SAFETY: `row < len`, entao a origem contem um valor valido; o
            // contrato garante que `dst` e um destino valido do mesmo tamanho.
            // As duas regioes pertencem a alocacoes distintas.
            unsafe { std::ptr::copy_nonoverlapping(self.ptr_at(row), dst, size) };
        }
        self.fecha_buraco(row)
    }

    /// Traz o ultimo elemento para `row`, deixando `row` inicializado de novo.
    ///
    /// Pressupoe que o valor que estava em `row` ja foi destruido ou movido.
    fn fecha_buraco(&mut self, row: usize) -> Option<usize> {
        let ultimo = self.len - 1;
        self.len = ultimo;

        if row == ultimo {
            return None;
        }

        let size = self.item_layout.size();
        if size > 0 {
            // SAFETY: `row` e `ultimo` sao menores que a capacidade e
            // diferentes entre si, entao as regioes de `size` bytes nao se
            // sobrepoem. `ultimo` continha um valor valido e passa a ser
            // considerado nao inicializado, ja que `len` encolheu.
            unsafe { std::ptr::copy_nonoverlapping(self.ptr_at(ultimo), self.ptr_at(row), size) };
        }
        Some(ultimo)
    }

    /// Destroi todos os valores, preservando a memoria alocada.
    pub(crate) fn clear(&mut self) {
        if let Some(drop) = self.drop {
            for row in 0..self.len {
                // SAFETY: `row < len`, entao cada slot percorrido contem um
                // valor inicializado. `len` vai a zero logo em seguida, entao
                // nenhum deles e destruido duas vezes.
                unsafe { drop(self.ptr_at(row)) };
            }
        }
        self.len = 0;
    }

    /// Referencia tipada ao valor em `row`.
    ///
    /// # Safety
    ///
    /// `row` precisa ser menor que `len` e `T` precisa ser o tipo desta coluna.
    /// O tempo de vida devolvido nao e verificado: quem chama garante que a
    /// coluna nao e modificada enquanto a referencia existir.
    #[inline]
    pub(crate) unsafe fn get<T: Component>(&self, row: usize) -> &T {
        debug_assert!(row < self.len);
        debug_assert_eq!(self.item_layout, Layout::new::<T>());
        // SAFETY: pelo contrato, `row < len` — logo o slot esta inicializado —
        // e `T` e o tipo da coluna, entao o ponteiro esta alinhado para `T`.
        unsafe { &*self.ptr_at(row).cast::<T>() }
    }

    /// Referencia mutavel tipada ao valor em `row`.
    ///
    /// # Safety
    ///
    /// As mesmas condicoes de [`get`](Self::get), e o valor nao pode estar
    /// emprestado em nenhum outro lugar.
    #[inline]
    pub(crate) unsafe fn get_mut<T: Component>(&mut self, row: usize) -> &mut T {
        debug_assert!(row < self.len);
        debug_assert_eq!(self.item_layout, Layout::new::<T>());
        // SAFETY: identico a `get`, com exclusividade garantida pelo `&mut
        // self` e pelo contrato da funcao.
        unsafe { &mut *self.ptr_at(row).cast::<T>() }
    }
}

impl std::fmt::Debug for Column {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column")
            .field("len", &self.len)
            .field("cap", &self.cap)
            .field("item_size", &self.item_layout.size())
            .finish()
    }
}

impl Drop for Column {
    fn drop(&mut self) {
        self.clear();

        if self.item_layout.size() > 0 && self.cap > 0 {
            // SAFETY: `self.data` foi obtido de `alloc`/`realloc` com
            // exatamente este layout, e ainda nao foi liberado. `clear` acabou
            // de destruir todos os valores.
            unsafe {
                std::alloc::dealloc(self.data.as_ptr(), layout_de_array(self.item_layout, self.cap))
            };
        }
    }
}

/// Layout de `n` elementos contiguos de `item`.
///
/// Rust garante que `size_of::<T>()` e multiplo de `align_of::<T>()`, entao
/// multiplicar o tamanho pela contagem descreve o array sem precisar de padding
/// adicional.
fn layout_de_array(item: Layout, n: usize) -> Layout {
    let size = item.size().checked_mul(n).expect("tamanho de coluna estourou usize");
    Layout::from_size_align(size, item.align()).expect("layout de coluna invalido")
}

/// Ponteiro pendurado, porem alinhado para `layout`.
///
/// Usado enquanto a coluna nao alocou nada e para colunas de ZST. Nunca e
/// desreferenciado para ler bytes, mas precisa estar alinhado porque vira
/// referencia `&T` no caso ZST.
fn dangling_alinhado(layout: Layout) -> NonNull<u8> {
    NonNull::new(std::ptr::without_provenance_mut(layout.align()))
        .expect("alinhamento de um Layout nunca e zero")
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::component::Components;

    fn coluna_de<T: Component>(c: &mut Components) -> Column {
        let id = c.register::<T>();
        Column::new(c.info(id).unwrap())
    }

    #[test]
    fn empurra_e_le() {
        let mut c = Components::new();
        let mut col = coluna_de::<u32>(&mut c);

        col.push(10_u32);
        col.push(20_u32);

        assert_eq!(col.len(), 2);
        // SAFETY: duas linhas foram inseridas e o tipo confere.
        unsafe {
            assert_eq!(*col.get::<u32>(0), 10);
            assert_eq!(*col.get::<u32>(1), 20);
        }
    }

    #[test]
    fn swap_remove_traz_o_ultimo_para_o_buraco() {
        let mut c = Components::new();
        let mut col = coluna_de::<u32>(&mut c);
        for v in [1_u32, 2, 3, 4] {
            col.push(v);
        }

        assert_eq!(col.swap_remove_drop(1), Some(3));
        assert_eq!(col.len(), 3);
        // SAFETY: tres linhas validas, tipo confere.
        unsafe {
            assert_eq!(*col.get::<u32>(0), 1);
            assert_eq!(*col.get::<u32>(1), 4, "o ultimo deveria ter ocupado a linha 1");
            assert_eq!(*col.get::<u32>(2), 3);
        }
    }

    #[test]
    fn remover_o_ultimo_nao_move_ninguem() {
        let mut c = Components::new();
        let mut col = coluna_de::<u32>(&mut c);
        col.push(1_u32);
        col.push(2_u32);

        assert_eq!(col.swap_remove_drop(1), None);
        assert_eq!(col.len(), 1);
    }

    #[test]
    fn cresce_geometricamente() {
        let mut c = Components::new();
        let mut col = coluna_de::<u64>(&mut c);

        for i in 0..1_000_u64 {
            col.push(i);
        }

        assert_eq!(col.len(), 1_000);
        assert!(col.capacity() >= 1_000);
        // SAFETY: mil linhas validas, tipo confere.
        unsafe {
            assert_eq!(*col.get::<u64>(0), 0);
            assert_eq!(*col.get::<u64>(999), 999);
        }
    }

    /// Sentinela que registra a propria destruicao.
    ///
    /// Usa `Arc<Mutex<_>>` e nao `Rc<RefCell<_>>` porque `Component` exige
    /// `Send + Sync`.
    struct Sentinela(Arc<Mutex<Vec<u32>>>, u32);

    impl Drop for Sentinela {
        fn drop(&mut self) {
            self.0.lock().unwrap().push(self.1);
        }
    }

    fn destruidos(registro: &Arc<Mutex<Vec<u32>>>) -> Vec<u32> {
        registro.lock().unwrap().clone()
    }

    #[test]
    fn destroi_os_valores_ao_limpar() {
        let registro = Arc::new(Mutex::new(Vec::new()));
        let mut c = Components::new();
        let mut col = coluna_de::<Sentinela>(&mut c);

        for i in 0..3 {
            col.push(Sentinela(Arc::clone(&registro), i));
        }

        assert!(destruidos(&registro).is_empty(), "nada deveria ter sido destruido ainda");
        col.clear();
        assert_eq!(destruidos(&registro), vec![0, 1, 2]);
        assert!(col.is_empty());
    }

    #[test]
    fn drop_da_coluna_destroi_o_que_restou() {
        let registro = Arc::new(Mutex::new(Vec::new()));

        {
            let mut c = Components::new();
            let mut col = coluna_de::<Sentinela>(&mut c);
            for i in 0..2 {
                col.push(Sentinela(Arc::clone(&registro), i));
            }
        }

        assert_eq!(destruidos(&registro), vec![0, 1], "o Drop da coluna deveria limpar tudo");
    }

    #[test]
    fn swap_remove_drop_destroi_exatamente_um() {
        let registro = Arc::new(Mutex::new(Vec::new()));
        let mut c = Components::new();
        let mut col = coluna_de::<Sentinela>(&mut c);

        for i in 0..3 {
            col.push(Sentinela(Arc::clone(&registro), i));
        }

        col.swap_remove_drop(0);
        assert_eq!(destruidos(&registro), vec![0]);

        // O elemento 2 foi movido para a linha 0; nenhuma destruicao dupla.
        col.clear();
        assert_eq!(destruidos(&registro), vec![0, 2, 1]);
    }

    #[test]
    fn swap_remove_move_transfere_a_posse() {
        let mut c = Components::new();
        let mut col = coluna_de::<String>(&mut c);
        col.push(String::from("primeiro"));
        col.push(String::from("segundo"));

        let mut destino = ManuallyDrop::new(String::new());
        // SAFETY: linha 0 e valida e `destino` tem o tamanho e o alinhamento de
        // `String`. A posse passa para `destino`, que e destruido abaixo.
        let movida = unsafe { col.swap_remove_move(0, (&raw mut destino).cast::<u8>()) };

        assert_eq!(movida, Some(1));
        assert_eq!(&*destino, "primeiro");
        assert_eq!(col.len(), 1);
        // SAFETY: uma linha valida, tipo confere.
        unsafe { assert_eq!(col.get::<String>(0), "segundo") };

        // O valor movido pertence ao teste agora.
        let _ = ManuallyDrop::into_inner(destino);
    }

    #[test]
    fn zst_nao_aloca_e_conta_certo() {
        let mut c = Components::new();
        let mut col = coluna_de::<()>(&mut c);

        for _ in 0..100 {
            col.push(());
        }

        assert_eq!(col.len(), 100);
        assert_eq!(col.swap_remove_drop(0), Some(99));
        assert_eq!(col.len(), 99);
    }
}
