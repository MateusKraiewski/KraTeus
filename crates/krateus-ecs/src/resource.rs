//! Dados unicos do mundo, nao ligados a nenhuma entidade.
//!
//! Tempo, gravidade, estado de entrada, configuracao de qualidade grafica: sao
//! coisas das quais existe uma so instancia. Modelar isso como componente de uma
//! entidade-singleton funciona, mas mente sobre a estrutura e obriga cada
//! sistema a saber qual e a entidade certa.
//!
//! Um recurso pode estar **registrado sem estar presente**. A separacao importa:
//! o scheduler precisa calcular o acesso de um sistema a partir dos tipos, antes
//! de qualquer valor existir. Registrar so cria o identificador; inserir e que
//! coloca o valor.

use std::any::{Any, TypeId, type_name};
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::fmt;
use std::ptr::NonNull;

/// Tipo que pode ser guardado como recurso.
///
/// `Send + Sync` porque o scheduler vai executar sistemas em paralelo, e
/// `'static` por causa do `TypeId`. A implementacao e automatica.
pub trait Resource: Send + Sync + 'static {}

impl<T: Send + Sync + 'static> Resource for T {}

/// Identificador denso de um tipo de recurso.
///
/// Vive num espaco proprio, separado do de [`ComponentId`](crate::ComponentId):
/// um componente e um recurso do mesmo tipo sao coisas distintas e nao
/// conflitam entre si.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ResourceId(u32);

impl ResourceId {
    /// Valor bruto do identificador.
    #[inline]
    #[must_use]
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}

/// Colecao de recursos de um [`World`](crate::World).
#[derive(Default)]
pub struct Resources {
    /// Indexado por [`ResourceId`]. `None` quando o tipo foi registrado mas
    /// nenhum valor foi inserido.
    ///
    /// O `UnsafeCell` existe por causa do scheduler. Dois sistemas que escrevem
    /// recursos diferentes rodam em paralelo, cada um segurando apenas um
    /// `&Resources` — e derivar `&mut` de uma referencia compartilhada e
    /// aliasing invalido, ainda que os alvos sejam disjuntos. `UnsafeCell` e a
    /// ferramenta exata para "referencia compartilhada, exclusividade provada
    /// por fora"; aqui, provada pelo [`Access`](crate::Access).
    ///
    /// Os slots sao independentes entre si, entao a exclusividade e por recurso,
    /// nao pela colecao.
    slots: Vec<Option<UnsafeCell<Box<dyn Any + Send + Sync>>>>,
    nomes: Vec<&'static str>,
    indices: HashMap<TypeId, ResourceId>,
    presentes: usize,
}

// SAFETY: o `UnsafeCell` dos slots nao cria mutabilidade compartilhada
// acessivel pela API segura: mutar exige `&mut self`, e com apenas
// `&Resources` so se obtem `&R`. A unica porta para `&mut R` a partir de
// `&self` e `get_unchecked_mut`, que e `unsafe` e transfere a obrigacao para
// quem chama.
unsafe impl Sync for Resources {}

impl Resources {
    /// Cria uma colecao vazia.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Quantidade de recursos com valor presente.
    #[inline]
    #[must_use]
    pub const fn len(&self) -> usize {
        self.presentes
    }

    /// Indica se nenhum recurso tem valor.
    #[inline]
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.presentes == 0
    }

    /// Quantidade de tipos registrados, com ou sem valor.
    #[inline]
    #[must_use]
    pub fn registered(&self) -> usize {
        self.slots.len()
    }

    /// Devolve o identificador de `R`, registrando o tipo se for a primeira vez.
    ///
    /// Nao insere valor nenhum.
    pub fn register<R: Resource>(&mut self) -> ResourceId {
        let type_id = TypeId::of::<R>();
        if let Some(&id) = self.indices.get(&type_id) {
            return id;
        }

        let id = ResourceId(self.slots.len() as u32);
        self.slots.push(None);
        self.nomes.push(type_name::<R>());
        self.indices.insert(type_id, id);
        id
    }

    /// Identificador de `R`, se ja registrado.
    #[must_use]
    pub fn id<R: Resource>(&self) -> Option<ResourceId> {
        self.indices.get(&TypeId::of::<R>()).copied()
    }

    /// Nome do tipo de um recurso registrado, para diagnostico.
    #[must_use]
    pub fn name(&self, id: ResourceId) -> Option<&'static str> {
        self.nomes.get(id.index()).copied()
    }

    /// Insere o valor, devolvendo o anterior se havia um.
    pub fn insert<R: Resource>(&mut self, valor: R) -> Option<R> {
        let id = self.register::<R>();
        let anterior = self.slots[id.index()].replace(UnsafeCell::new(Box::new(valor)));

        match anterior {
            Some(caixa) => {
                Some(*caixa.into_inner().downcast::<R>().expect("slot guarda o tipo do seu id"))
            }
            None => {
                self.presentes += 1;
                None
            }
        }
    }

    /// Remove o valor e o devolve.
    pub fn remove<R: Resource>(&mut self) -> Option<R> {
        let id = self.id::<R>()?;
        let caixa = self.slots[id.index()].take()?;
        self.presentes -= 1;
        Some(*caixa.into_inner().downcast::<R>().expect("slot guarda o tipo do seu id"))
    }

    /// Indica se `R` tem valor presente.
    #[must_use]
    pub fn contains<R: Resource>(&self) -> bool {
        self.id::<R>().is_some_and(|id| self.slots[id.index()].is_some())
    }

    /// Referencia ao valor de `R`.
    #[must_use]
    pub fn get<R: Resource>(&self) -> Option<&R> {
        let id = self.id::<R>()?;
        let celula = self.slots[id.index()].as_ref()?;
        // SAFETY: `&self` garante que ninguem tem `&mut` a esta colecao pela
        // API segura. A unica forma de haver um `&mut R` vivo aqui seria por
        // `get_unchecked_mut`, cujo contrato proibe exatamente isso.
        unsafe { &*celula.get() }.downcast_ref::<R>()
    }

    /// Referencia mutavel ao valor de `R`.
    pub fn get_mut<R: Resource>(&mut self) -> Option<&mut R> {
        let id = self.id::<R>()?;
        self.slots[id.index()].as_mut()?.get_mut().downcast_mut::<R>()
    }

    /// Ponteiro para o valor de `R`, obtido a partir de `&self`.
    ///
    /// Existe para o scheduler: dois sistemas que escrevem recursos distintos
    /// rodam em paralelo segurando apenas `&Resources`, e a disjuncao entre eles
    /// e provada pelo [`Access`](crate::Access), nao pelo compilador.
    ///
    /// Devolve **ponteiro e nao referencia** de proposito. Um `&mut R` afirma
    /// exclusividade, e esta funcao nao tem como sustentar essa afirmacao — quem
    /// a sustenta e quem conhece o `Access`. Devolver ponteiro deixa a asercao
    /// no ponto onde ela pode ser justificada.
    ///
    /// # Safety
    ///
    /// O slot precisa nao estar sendo acessado por mais ninguem no momento da
    /// chamada. Recursos de tipos diferentes ocupam slots independentes e nao se
    /// afetam.
    pub(crate) unsafe fn get_ptr<R: Resource>(&self) -> Option<NonNull<R>> {
        let id = self.id::<R>()?;
        let celula = self.slots[id.index()].as_ref()?;

        // SAFETY: o contrato transfere para quem chama a garantia de que este
        // slot esta livre. A referencia criada aqui serve apenas para resolver o
        // downcast e nao sobrevive a chamada: vira ponteiro na linha seguinte.
        let valor = unsafe { &mut *celula.get() }.downcast_mut::<R>()?;
        Some(NonNull::from(valor))
    }

    /// Itera sobre os recursos registrados, em ordem de registro.
    ///
    /// Percorre o vetor, e nao o mapa de tipos: a ordem de um `HashMap` varia
    /// entre execucoes, e a secao 15 do documento de visao exige iteracao
    /// estavel.
    pub fn iter(&self) -> impl Iterator<Item = (ResourceId, &'static str, bool)> {
        self.nomes
            .iter()
            .enumerate()
            .map(|(i, &nome)| (ResourceId(i as u32), nome, self.slots[i].is_some()))
    }
}

impl fmt::Debug for Resources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `dyn Any` nao e `Debug`; mostra o que da para mostrar.
        f.debug_struct("Resources")
            .field("registrados", &self.slots.len())
            .field("presentes", &self.presentes)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Gravidade(f32);

    #[derive(Debug, PartialEq)]
    struct Contador(u32);

    #[test]
    fn insere_e_le() {
        let mut r = Resources::new();
        r.insert(Gravidade(-9.81));

        assert_eq!(r.get::<Gravidade>(), Some(&Gravidade(-9.81)));
        assert_eq!(r.len(), 1);
        assert!(r.contains::<Gravidade>());
    }

    #[test]
    fn recurso_ausente_devolve_none() {
        let r = Resources::new();
        assert_eq!(r.get::<Gravidade>(), None);
        assert!(!r.contains::<Gravidade>());
    }

    #[test]
    fn insercao_devolve_o_valor_anterior() {
        let mut r = Resources::new();
        assert_eq!(r.insert(Contador(1)), None);
        assert_eq!(r.insert(Contador(2)), Some(Contador(1)));
        assert_eq!(r.get::<Contador>(), Some(&Contador(2)));
        assert_eq!(r.len(), 1, "substituir nao cria um segundo recurso");
    }

    #[test]
    fn mutacao_persiste() {
        let mut r = Resources::new();
        r.insert(Contador(0));
        r.get_mut::<Contador>().unwrap().0 += 5;
        assert_eq!(r.get::<Contador>(), Some(&Contador(5)));
    }

    #[test]
    fn remove_devolve_o_valor_e_esvazia_o_slot() {
        let mut r = Resources::new();
        r.insert(Gravidade(-1.0));

        assert_eq!(r.remove::<Gravidade>(), Some(Gravidade(-1.0)));
        assert!(!r.contains::<Gravidade>());
        assert!(r.is_empty());
        assert_eq!(r.remove::<Gravidade>(), None, "a segunda remocao e inofensiva");
    }

    #[test]
    fn registrar_nao_insere_valor() {
        let mut r = Resources::new();
        let id = r.register::<Gravidade>();

        assert_eq!(r.registered(), 1);
        assert!(r.is_empty(), "registrar cria o identificador, nao o valor");
        assert_eq!(r.id::<Gravidade>(), Some(id));
        assert!(!r.contains::<Gravidade>());
    }

    #[test]
    fn registrar_duas_vezes_devolve_o_mesmo_id() {
        let mut r = Resources::new();
        assert_eq!(r.register::<Gravidade>(), r.register::<Gravidade>());
        assert_eq!(r.registered(), 1);
    }

    #[test]
    fn tipos_distintos_recebem_ids_distintos() {
        let mut r = Resources::new();
        assert_ne!(r.register::<Gravidade>(), r.register::<Contador>());
    }

    #[test]
    fn remover_e_reinserir_reaproveita_o_id() {
        let mut r = Resources::new();
        r.insert(Contador(1));
        let id = r.id::<Contador>().unwrap();

        r.remove::<Contador>();
        r.insert(Contador(9));

        assert_eq!(r.id::<Contador>(), Some(id));
        assert_eq!(r.registered(), 1);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn iteracao_segue_a_ordem_de_registro() {
        let mut r = Resources::new();
        r.insert(Gravidade(0.0));
        r.register::<Contador>();

        let vistos: Vec<_> = r.iter().map(|(_, nome, presente)| (nome, presente)).collect();

        assert_eq!(vistos.len(), 2);
        assert!(vistos[0].0.contains("Gravidade"));
        assert!(vistos[0].1, "gravidade tem valor");
        assert!(vistos[1].0.contains("Contador"));
        assert!(!vistos[1].1, "contador foi so registrado");
    }

    #[test]
    fn name_identifica_o_tipo() {
        let mut r = Resources::new();
        let id = r.register::<Gravidade>();
        assert!(r.name(id).unwrap().contains("Gravidade"));
    }
}
