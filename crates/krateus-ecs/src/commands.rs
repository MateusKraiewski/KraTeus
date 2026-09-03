//! Mudancas estruturais enfileiradas para aplicacao posterior.
//!
//! Uma query segura ponteiros para as colunas dos archetypes que percorre.
//! Criar ou destruir entidade no meio da iteracao invalidaria esses ponteiros:
//! a coluna pode realocar, e o `swap_remove` move outra entidade para a linha
//! que esta sendo lida. O emprestimo do `World` ja impede isso em tempo de
//! compilacao — o que falta e uma forma de expressar a intencao.
//!
//! ```
//! use krateus_ecs::{Commands, Entity, World};
//!
//! struct Vida(i32);
//!
//! let mut world = World::new();
//! world.spawn((Vida(10),));
//! world.spawn((Vida(0),));
//!
//! let mut commands = Commands::new();
//! for (e, vida) in world.query::<(Entity, &Vida)>() {
//!     if vida.0 <= 0 {
//!         commands.despawn(e);
//!     }
//! }
//! commands.apply(&mut world);
//!
//! assert_eq!(world.len(), 1);
//! ```
//!
//! # Ordem
//!
//! Os comandos sao aplicados na ordem em que foram enfileirados. Isso e
//! requisito, nao conveniencia: a secao 15 do documento de visao exige que a
//! mesma sequencia de entrada produza o mesmo estado, e uma ordem de aplicacao
//! instavel quebraria replay e networking deterministico.
//!
//! # Alocacao
//!
//! `despawn` e `remove` nao alocam: viram um ponteiro de funcao monomorfizado
//! mais a entidade. Sao os que aparecem em laco sobre muitas entidades, que e o
//! caso que a secao 11 pede para nao alocar.
//!
//! `spawn` e `insert` carregam dados de tamanho arbitrario e alocam uma vez por
//! comando. Um buffer de bytes type-erased evitaria isso, ao custo de mais
//! `unsafe` fora do modulo que hoje o concentra. Fica para quando houver
//! benchmark mostrando que importa — a secao 19 pede medir antes de otimizar.

use crate::bundle::Bundle;
use crate::component::Component;
use crate::entity::Entity;
use crate::resource::Resource;
use crate::world::World;

/// Uma mudanca estrutural pendente.
enum Comando {
    /// Opera sobre o mundo sem dado nenhum alem do ponteiro de funcao.
    Simples(fn(&mut World)),
    /// Opera sobre uma entidade. Cobre `despawn` e remocao de componente sem
    /// alocar: o tipo vira monomorfizacao, nao dado.
    ComEntidade(Entity, fn(&mut World, Entity)),
    /// Carrega dados de tamanho arbitrario: `spawn`, `insert`, insercao de
    /// recurso.
    ComDados(Box<dyn FnOnce(&mut World) + Send + Sync>),
}

/// Fila de mudancas estruturais.
///
/// Nao guarda referencia ao mundo, entao pode ser preenchida enquanto uma query
/// esta iterando e aplicada depois, quando o emprestimo tiver terminado.
#[derive(Default)]
pub struct Commands {
    comandos: Vec<Comando>,
}

impl Commands {
    /// Cria uma fila vazia.
    #[must_use]
    pub const fn new() -> Self {
        Self { comandos: Vec::new() }
    }

    /// Cria uma fila com capacidade reservada.
    ///
    /// Preferir quando a ordem de grandeza e conhecida: um sistema que despawna
    /// milhares de entidades por passo evita a cascata de realocacoes.
    #[must_use]
    pub fn with_capacity(n: usize) -> Self {
        Self { comandos: Vec::with_capacity(n) }
    }

    /// Quantidade de comandos pendentes.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.comandos.len()
    }

    /// Indica se nao ha comandos pendentes.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.comandos.is_empty()
    }

    /// Descarta os comandos pendentes sem aplicar.
    pub fn clear(&mut self) {
        self.comandos.clear();
    }

    /// Enfileira a criacao de uma entidade com os componentes do bundle.
    ///
    /// Nao devolve a [`Entity`] criada: o identificador so existe na aplicacao.
    /// Ver a decisao D11 em `docs/DECISOES.md` para o porque e para o desenho da
    /// reserva de identificadores.
    pub fn spawn<B: Bundle>(&mut self, bundle: B) {
        self.comandos.push(Comando::ComDados(Box::new(move |world: &mut World| {
            world.spawn(bundle);
        })));
    }

    /// Enfileira a destruicao de uma entidade.
    ///
    /// Aplicar sobre uma entidade que ja nao existe e inofensivo — o caso e
    /// comum, ja que dois sistemas do mesmo passo podem decidir remover a mesma
    /// entidade.
    pub fn despawn(&mut self, entity: Entity) {
        self.comandos.push(Comando::ComEntidade(entity, |world, e| {
            world.despawn(e);
        }));
    }

    /// Enfileira a insercao de componentes numa entidade existente.
    ///
    /// Se a entidade tiver sido destruida antes da aplicacao, o comando nao faz
    /// nada.
    pub fn insert<B: Bundle>(&mut self, entity: Entity, bundle: B) {
        self.comandos.push(Comando::ComDados(Box::new(move |world: &mut World| {
            world.insert(entity, bundle);
        })));
    }

    /// Enfileira a remocao do componente `T` de uma entidade.
    ///
    /// O valor removido e descartado. Para recupera-lo, use
    /// [`World::remove`](crate::World::remove) fora da iteracao.
    pub fn remove<T: Component>(&mut self, entity: Entity) {
        // A monomorfizacao carrega o tipo; a closure nao captura nada e por isso
        // vira um ponteiro de funcao, sem alocacao.
        fn aplicar<T: Component>(world: &mut World, entity: Entity) {
            world.remove::<T>(entity);
        }
        self.comandos.push(Comando::ComEntidade(entity, aplicar::<T>));
    }

    /// Enfileira a insercao de um recurso.
    pub fn insert_resource<R: Resource>(&mut self, valor: R) {
        self.comandos.push(Comando::ComDados(Box::new(move |world: &mut World| {
            world.insert_resource(valor);
        })));
    }

    /// Enfileira a remocao de um recurso.
    pub fn remove_resource<R: Resource>(&mut self) {
        fn aplicar<R: Resource>(world: &mut World) {
            world.remove_resource::<R>();
        }
        self.comandos.push(Comando::Simples(aplicar::<R>));
    }

    /// Enfileira uma operacao arbitraria sobre o mundo.
    ///
    /// Valvula de escape para o que a API tipada nao cobre. Preferir os metodos
    /// especificos: eles dizem o que vai acontecer, e este nao.
    pub fn add(&mut self, f: impl FnOnce(&mut World) + Send + Sync + 'static) {
        self.comandos.push(Comando::ComDados(Box::new(f)));
    }

    /// Aplica os comandos na ordem em que foram enfileirados e esvazia a fila.
    ///
    /// A fila fica reutilizavel: a capacidade e preservada, o que evita realocar
    /// a cada passo de simulacao.
    pub fn apply(&mut self, world: &mut World) {
        for comando in self.comandos.drain(..) {
            match comando {
                Comando::Simples(f) => f(world),
                Comando::ComEntidade(entity, f) => f(world, entity),
                Comando::ComDados(f) => f(world),
            }
        }
    }
}

impl std::fmt::Debug for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // As closures nao sao `Debug`; o que interessa e quantas ha.
        f.debug_struct("Commands").field("pendentes", &self.comandos.len()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Entity, With};

    #[derive(Debug, PartialEq)]
    struct Posicao(f32);
    #[derive(Debug, PartialEq)]
    struct Velocidade(f32);
    #[derive(Debug, PartialEq)]
    struct Gravidade(f32);
    struct Morta;

    #[test]
    fn fila_nova_esta_vazia() {
        let c = Commands::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn despawn_durante_iteracao() {
        let mut w = World::new();
        w.spawn((Posicao(1.0),));
        w.spawn((Posicao(2.0),));
        w.spawn((Posicao(3.0),));

        let mut c = Commands::new();
        for (e, p) in w.query::<(Entity, &Posicao)>() {
            if p.0 > 1.5 {
                c.despawn(e);
            }
        }

        assert_eq!(w.len(), 3, "nada muda antes de aplicar");
        c.apply(&mut w);
        assert_eq!(w.len(), 1);
        assert_eq!(w.query::<&Posicao>().next(), Some(&Posicao(1.0)));
    }

    #[test]
    fn spawn_enfileirado_so_existe_apos_aplicar() {
        let mut w = World::new();
        let mut c = Commands::new();

        c.spawn((Posicao(7.0), Velocidade(1.0)));
        assert!(w.is_empty());

        c.apply(&mut w);
        assert_eq!(w.len(), 1);
        assert_eq!(w.query::<&Posicao>().next(), Some(&Posicao(7.0)));
    }

    #[test]
    fn insert_e_remove_de_componente() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0), Velocidade(5.0)));

        let mut c = Commands::new();
        c.insert(e, (Morta,));
        c.remove::<Velocidade>(e);
        c.apply(&mut w);

        assert!(w.has::<Morta>(e));
        assert!(!w.has::<Velocidade>(e));
        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(0.0)));
    }

    #[test]
    fn aplicar_esvazia_a_fila() {
        let mut w = World::new();
        let mut c = Commands::new();
        c.spawn((Posicao(1.0),));

        c.apply(&mut w);
        assert!(c.is_empty());

        // A segunda aplicacao nao repete o comando.
        c.apply(&mut w);
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn despawn_de_entidade_ja_morta_e_inofensivo() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0),));

        let mut c = Commands::new();
        c.despawn(e);
        c.despawn(e);
        c.apply(&mut w);

        assert!(w.is_empty());
    }

    #[test]
    fn insert_em_entidade_destruida_no_mesmo_lote_nao_entra_em_panico() {
        let mut w = World::new();
        let e = w.spawn((Posicao(1.0),));

        let mut c = Commands::new();
        c.despawn(e);
        c.insert(e, (Velocidade(1.0),));
        c.apply(&mut w);

        assert!(w.is_empty());
    }

    #[test]
    fn ordem_de_aplicacao_e_a_de_enfileiramento() {
        let mut w = World::new();
        let e = w.spawn((Posicao(0.0),));

        let mut c = Commands::new();
        c.insert(e, (Posicao(1.0),));
        c.insert(e, (Posicao(2.0),));
        c.insert(e, (Posicao(3.0),));
        c.apply(&mut w);

        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(3.0)), "o ultimo comando prevalece");
    }

    #[test]
    fn recursos_pela_fila() {
        let mut w = World::new();
        let mut c = Commands::new();

        c.insert_resource(Gravidade(-9.81));
        c.apply(&mut w);
        assert_eq!(w.get_resource::<Gravidade>(), Some(&Gravidade(-9.81)));

        c.remove_resource::<Gravidade>();
        c.apply(&mut w);
        assert!(!w.contains_resource::<Gravidade>());
    }

    #[test]
    fn valvula_de_escape_recebe_o_mundo() {
        let mut w = World::new();
        let mut c = Commands::new();

        c.add(|world: &mut World| {
            world.spawn((Posicao(42.0),));
        });
        c.apply(&mut w);

        assert_eq!(w.query::<&Posicao>().next(), Some(&Posicao(42.0)));
    }

    #[test]
    fn clear_descarta_sem_aplicar() {
        let mut w = World::new();
        let mut c = Commands::new();
        c.spawn((Posicao(1.0),));
        c.clear();
        c.apply(&mut w);

        assert!(w.is_empty());
    }

    #[test]
    fn ciclo_realista_de_um_passo_de_simulacao() {
        let mut w = World::new();
        for i in 0..10 {
            w.spawn((Posicao(i as f32), Velocidade(1.0)));
        }

        let mut c = Commands::with_capacity(16);

        // Um passo: integra, marca quem passou do limite, cria um substituto.
        for (e, p, v) in w.query::<(Entity, &mut Posicao, &Velocidade)>() {
            p.0 += v.0;
            if p.0 > 8.0 {
                c.insert(e, (Morta,));
                c.spawn((Posicao(0.0), Velocidade(1.0)));
            }
        }
        c.apply(&mut w);

        // Entidades 8 e 9 passaram de 8.0 apos a integracao.
        assert_eq!(w.query_filtered::<&Posicao, With<Morta>>().count(), 2);
        assert_eq!(w.len(), 12, "duas substitutas foram criadas");
    }

    #[test]
    fn a_fila_e_send_e_sync() {
        // O scheduler vai levar uma fila por sistema para outra thread.
        fn exige<T: Send + Sync>() {}
        exige::<Commands>();
    }

    #[test]
    fn mesma_sequencia_produz_o_mesmo_estado() {
        let executar = || {
            let mut w = World::new();
            let mut c = Commands::new();
            for i in 0..40 {
                c.spawn((Posicao(i as f32),));
            }
            c.apply(&mut w);

            let alvos: Vec<_> = w
                .query::<(Entity, &Posicao)>()
                .filter(|(_, p)| (p.0 as u32) % 3 == 0)
                .map(|(e, _)| e)
                .collect();
            for e in alvos {
                c.despawn(e);
            }
            c.apply(&mut w);

            w.query::<&Posicao>().map(|p| p.0.to_bits()).collect::<Vec<_>>()
        };

        assert_eq!(executar(), executar());
    }
}
