//! Registro de tipos de componente.
//!
//! O armazenamento por archetype guarda componentes em colunas type-erased, o
//! que exige conhecer, em tempo de execucao, o `Layout` de cada tipo e como
//! destrui-lo. Este modulo e quem mantem essa informacao, associando a cada
//! `TypeId` um [`ComponentId`] denso — um indice pequeno, barato de comparar e
//! de usar como chave em vetores.

use std::alloc::Layout;
use std::any::{TypeId, type_name};
use std::collections::HashMap;

/// Tipo que pode ser usado como componente.
///
/// O limite `Send + Sync` existe porque o scheduler da Fase 2 vai executar
/// sistemas em paralelo: um componente que nao pode cruzar threads tornaria
/// isso impossivel. `'static` e exigencia do `TypeId`.
///
/// A implementacao e automatica; nao ha nada para escrever.
pub trait Component: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Component for T {}

/// Indice denso que identifica um tipo de componente dentro de um
/// [`World`](crate::World).
///
/// Nao e estavel entre execucoes diferentes: depende da ordem de registro.
/// Persistir componentes exige mapear pelo nome, nao por este identificador.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(u32);

impl ComponentId {
    /// Valor bruto do identificador.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Tudo que o armazenamento precisa saber sobre um tipo de componente sem
/// conhecer o tipo em si.
#[derive(Debug, Clone)]
pub struct ComponentInfo {
    id: ComponentId,
    name: &'static str,
    type_id: TypeId,
    layout: Layout,
    drop: Option<unsafe fn(*mut u8)>,
}

impl ComponentInfo {
    /// Identificador denso deste componente.
    #[inline]
    #[must_use]
    pub const fn id(&self) -> ComponentId {
        self.id
    }

    /// Nome do tipo, para diagnostico.
    #[inline]
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// `TypeId` do tipo.
    #[inline]
    #[must_use]
    pub const fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Layout de memoria de um valor.
    #[inline]
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// Funcao de destruicao, ou `None` se o tipo nao precisa de `Drop`.
    ///
    /// Ausente para tipos triviais, o que permite a coluna pular o laco de
    /// destruicao inteiro ao limpar.
    #[inline]
    #[must_use]
    pub(crate) const fn drop_fn(&self) -> Option<unsafe fn(*mut u8)> {
        self.drop
    }
}

/// Destroi, no lugar, um valor de tipo `T` apontado por `ptr`.
///
/// # Safety
///
/// `ptr` precisa apontar para um valor de `T` valido, devidamente alinhado, que
/// nao seja usado depois desta chamada.
unsafe fn drop_in_place<T>(ptr: *mut u8) {
    // SAFETY: o contrato da funcao exige que `ptr` aponte para um `T` valido e
    // alinhado. `ComponentInfo` so associa esta funcao ao tipo que a gerou, e a
    // coluna so a chama sobre slots que sabe estarem inicializados.
    unsafe { ptr.cast::<T>().drop_in_place() }
}

/// Registro dos tipos de componente conhecidos por um `World`.
#[derive(Debug, Default)]
pub struct Components {
    infos: Vec<ComponentInfo>,
    indices: HashMap<TypeId, ComponentId>,
}

impl Components {
    /// Cria um registro vazio.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Quantidade de tipos registrados.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.infos.len()
    }

    /// Indica se nenhum tipo foi registrado.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.infos.is_empty()
    }

    /// Devolve o identificador de `T`, registrando o tipo se for a primeira vez.
    pub fn register<T: Component>(&mut self) -> ComponentId {
        let type_id = TypeId::of::<T>();
        if let Some(&id) = self.indices.get(&type_id) {
            return id;
        }

        let id = ComponentId(self.infos.len() as u32);
        self.infos.push(ComponentInfo {
            id,
            name: type_name::<T>(),
            type_id,
            layout: Layout::new::<T>(),
            // Tipos sem `Drop` nao precisam de funcao de destruicao. Guardar
            // `None` aqui deixa a coluna pular o laco inteiro.
            drop: std::mem::needs_drop::<T>().then_some(drop_in_place::<T> as unsafe fn(*mut u8)),
        });
        self.indices.insert(type_id, id);
        id
    }

    /// Identificador de `T`, se ele ja tiver sido registrado.
    #[must_use]
    pub fn id<T: Component>(&self) -> Option<ComponentId> {
        self.indices.get(&TypeId::of::<T>()).copied()
    }

    /// Informacao de um componente registrado.
    #[must_use]
    pub fn info(&self, id: ComponentId) -> Option<&ComponentInfo> {
        self.infos.get(id.index())
    }

    /// Itera sobre os componentes registrados, em ordem de registro.
    pub fn iter(&self) -> impl Iterator<Item = &ComponentInfo> {
        self.infos.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Posicao(#[allow(dead_code)] f32);
    struct ComDrop(#[allow(dead_code)] String);

    #[test]
    fn registrar_duas_vezes_devolve_o_mesmo_id() {
        let mut c = Components::new();
        let a = c.register::<Posicao>();
        let b = c.register::<Posicao>();
        assert_eq!(a, b);
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn tipos_distintos_recebem_ids_distintos() {
        let mut c = Components::new();
        let p = c.register::<Posicao>();
        let d = c.register::<ComDrop>();
        assert_ne!(p, d);
        assert_eq!(c.len(), 2);
    }

    #[test]
    fn id_so_existe_apos_registro() {
        let mut c = Components::new();
        assert_eq!(c.id::<Posicao>(), None);
        let id = c.register::<Posicao>();
        assert_eq!(c.id::<Posicao>(), Some(id));
    }

    #[test]
    fn drop_e_omitido_para_tipos_triviais() {
        let mut c = Components::new();
        let trivial = c.register::<u64>();
        let com_drop = c.register::<ComDrop>();

        assert!(c.info(trivial).unwrap().drop_fn().is_none());
        assert!(c.info(com_drop).unwrap().drop_fn().is_some());
    }

    #[test]
    fn info_carrega_layout_do_tipo() {
        let mut c = Components::new();
        let id = c.register::<u64>();
        let info = c.info(id).unwrap();
        assert_eq!(info.layout(), Layout::new::<u64>());
        assert!(info.name().contains("u64"));
    }
}
