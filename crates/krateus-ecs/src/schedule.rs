//! Organizacao e execucao dos sistemas.
//!
//! O modelo e hibrido, e as duas metades resolvem problemas diferentes:
//!
//! - **Etapas sao declaradas**, na ordem em que o projeto quiser — `Input`,
//!   `Simulacao`, `Fisica`, `Extracao`, `Render`. Sao fronteiras de efeito
//!   previsiveis: tudo da etapa anterior terminou e teve seus comandos
//!   aplicados antes de a proxima comecar. E disso que replay e depuracao
//!   dependem.
//! - **O paralelismo dentro da etapa e calculado**, a partir do
//!   [`Access`](crate::Access) de cada sistema. Ninguem precisa dizer o que
//!   roda junto; isso sai dos tipos dos parametros (secao 5 do documento de
//!   visao).
//!
//! ```
//! use krateus_ecs::{Query, ResMut, Schedule, World};
//!
//! struct Posicao(f32);
//! struct Velocidade(f32);
//! struct Quadros(u32);
//!
//! fn mover(mut q: Query<(&mut Posicao, &Velocidade)>) {
//!     for (p, v) in q.iter() {
//!         p.0 += v.0;
//!     }
//! }
//!
//! fn contar(mut n: ResMut<Quadros>) {
//!     n.0 += 1;
//! }
//!
//! let mut world = World::new();
//! world.insert_resource(Quadros(0));
//! world.spawn((Posicao(0.0), Velocidade(1.0)));
//!
//! let mut schedule = Schedule::new();
//! schedule.add_stage("simulacao");
//! schedule.add_system("simulacao", mover);
//! schedule.add_system("simulacao", contar);
//! schedule.initialize(&mut world);
//!
//! // Os dois nao conflitam, entao caem na mesma subetapa.
//! assert_eq!(schedule.batches("simulacao").unwrap().len(), 1);
//!
//! schedule.run(&mut world);
//! assert_eq!(world.resource::<Quadros>().0, 1);
//! ```
//!
//! # Como as subetapas saem
//!
//! Dentro de uma etapa, cada sistema vai para a subetapa mais cedo possivel que
//! ainda respeite a ordem de insercao entre sistemas que conflitam:
//!
//! > subetapa(i) = 1 + maior subetapa entre os sistemas anteriores que conflitam
//! > com `i`, ou 0 se nenhum conflita.
//!
//! Isso preserva a ordem relativa de quem depende um do outro e paraleliza todo
//! o resto. Um agrupamento guloso — "coloque no primeiro grupo que couber" —
//! seria mais compacto e estaria errado: um sistema tardio poderia cair num
//! grupo anterior e passar na frente de um sistema com que conflita.
//!
//! # Determinismo
//!
//! A ordem de execucao dentro de uma subetapa varia, porque e paralela. O
//! resultado nao varia: sistemas na mesma subetapa tem acessos disjuntos, logo
//! seus efeitos comutam. Os command buffers, esses sim capazes de se
//! influenciar, sao aplicados sequencialmente na ordem de insercao.

use std::collections::HashMap;

use krateus_core::jobs::JobPool;

use crate::access::Access;
use crate::system::{IntoSystem, System};
use crate::world::World;
use crate::world_cell::WorldCell;

struct Etapa {
    nome: &'static str,
    sistemas: Vec<Box<dyn System>>,
    /// Indices em `sistemas`, agrupados por subetapa.
    subetapas: Vec<Vec<usize>>,
}

impl Etapa {
    /// Recalcula as subetapas a partir dos acessos ja resolvidos.
    fn agrupar(&mut self) {
        self.subetapas.clear();
        let mut nivel_de: Vec<usize> = Vec::with_capacity(self.sistemas.len());

        for i in 0..self.sistemas.len() {
            let acesso = self.sistemas[i].access();
            let mut nivel = 0;

            for (j, anterior) in nivel_de.iter().enumerate() {
                if !self.sistemas[j].access().compatible_with(acesso) {
                    nivel = nivel.max(anterior + 1);
                }
            }

            nivel_de.push(nivel);
            if self.subetapas.len() <= nivel {
                self.subetapas.resize_with(nivel + 1, Vec::new);
            }
            self.subetapas[nivel].push(i);
        }
    }
}

/// Conjunto ordenado de etapas, cada uma com seus sistemas.
#[derive(Default)]
pub struct Schedule {
    etapas: Vec<Etapa>,
    indice: HashMap<&'static str, usize>,
    inicializado: bool,
}

impl Schedule {
    /// Cria um schedule sem etapas.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Acrescenta uma etapa depois das existentes.
    ///
    /// # Panics
    ///
    /// Se ja houver etapa com este nome.
    pub fn add_stage(&mut self, nome: &'static str) -> &mut Self {
        assert!(
            !self.indice.contains_key(nome),
            "etapa {nome} ja existe; nomes de etapa precisam ser unicos"
        );
        self.indice.insert(nome, self.etapas.len());
        self.etapas.push(Etapa { nome, sistemas: Vec::new(), subetapas: Vec::new() });
        self
    }

    /// Acrescenta um sistema ao fim de uma etapa.
    ///
    /// A posicao importa: e o desempate estavel entre sistemas que conflitam.
    ///
    /// # Panics
    ///
    /// Se a etapa nao existir.
    pub fn add_system<M, S: IntoSystem<M>>(
        &mut self,
        etapa: &'static str,
        sistema: S,
    ) -> &mut Self {
        let i = *self
            .indice
            .get(etapa)
            .unwrap_or_else(|| panic!("etapa {etapa} nao existe; declare-a com add_stage antes"));

        self.etapas[i].sistemas.push(Box::new(sistema.into_system()));
        self.inicializado = false;
        self
    }

    /// Nomes das etapas, na ordem de execucao.
    #[must_use]
    pub fn stages(&self) -> Vec<&'static str> {
        self.etapas.iter().map(|e| e.nome).collect()
    }

    /// Resolve os parametros de todos os sistemas e calcula as subetapas.
    ///
    /// Precisa preceder qualquer execucao. Chamar de novo e barato e necessario
    /// depois de acrescentar sistemas.
    pub fn initialize(&mut self, world: &mut World) {
        for etapa in &mut self.etapas {
            for sistema in &mut etapa.sistemas {
                sistema.initialize(world);
            }
            etapa.agrupar();
        }
        self.inicializado = true;
    }

    /// Como os sistemas de uma etapa ficaram agrupados.
    ///
    /// Cada subetapa e uma lista de indices na ordem de insercao. Serve a
    /// diagnostico e a teste: e assim que se confere que a paralelizacao saiu
    /// como esperado.
    #[must_use]
    pub fn batches(&self, etapa: &'static str) -> Option<&[Vec<usize>]> {
        let i = *self.indice.get(etapa)?;
        Some(&self.etapas[i].subetapas)
    }

    /// Como [`batches`](Self::batches), com os nomes dos sistemas.
    #[must_use]
    pub fn batch_names(&self, etapa: &'static str) -> Option<Vec<Vec<&'static str>>> {
        let i = *self.indice.get(etapa)?;
        let etapa = &self.etapas[i];
        Some(
            etapa
                .subetapas
                .iter()
                .map(|sub| sub.iter().map(|&s| etapa.sistemas[s].name()).collect())
                .collect(),
        )
    }

    /// Acesso combinado de uma etapa inteira.
    #[must_use]
    pub fn stage_access(&self, etapa: &'static str) -> Option<Access> {
        let i = *self.indice.get(etapa)?;
        let mut acesso = Access::new();
        for sistema in &self.etapas[i].sistemas {
            acesso.extend(sistema.access());
        }
        Some(acesso)
    }

    /// Executa um passo com tudo em uma unica thread.
    ///
    /// A ordem e a de insercao, etapa por etapa. Serve para reproduzir um bug
    /// sem concorrencia e como referencia do que a versao paralela precisa
    /// produzir.
    ///
    /// # Panics
    ///
    /// Se [`initialize`](Self::initialize) nao tiver sido chamado desde o
    /// ultimo `add_system`.
    pub fn run(&mut self, world: &mut World) {
        self.exigir_inicializado();

        for etapa in &mut self.etapas {
            for i in 0..etapa.sistemas.len() {
                let cell = WorldCell::new(world);
                // SAFETY: execucao sequencial — nenhum outro sistema esta em
                // andamento, entao nao ha conflito possivel.
                unsafe { etapa.sistemas[i].run(cell) };
            }
            aplicar_diferidos(etapa, world);
        }
    }

    /// Executa um passo, paralelizando dentro de cada subetapa.
    ///
    /// # Panics
    ///
    /// Se [`initialize`](Self::initialize) nao tiver sido chamado desde o
    /// ultimo `add_system`.
    pub fn run_parallel(&mut self, world: &mut World, pool: &JobPool) {
        self.exigir_inicializado();

        for etapa in &mut self.etapas {
            for si in 0..etapa.subetapas.len() {
                let cell = WorldCell::new(world);
                let alvo = &etapa.subetapas[si];

                // Emprestimos mutaveis disjuntos: cada sistema aparece em
                // exatamente uma subetapa, entao os indices nao se repetem.
                let selecionados: Vec<&mut Box<dyn System>> = etapa
                    .sistemas
                    .iter_mut()
                    .enumerate()
                    .filter(|(i, _)| alvo.contains(i))
                    .map(|(_, s)| s)
                    .collect();

                pool.scope(|escopo| {
                    for sistema in selecionados {
                        escopo.spawn(move || {
                            // SAFETY: os sistemas desta subetapa tem acessos
                            // mutuamente compativeis — foi essa a condicao para
                            // ficarem juntos. Nenhum sistema de outra subetapa
                            // roda ao mesmo tempo, porque `scope` so retorna
                            // quando todos terminam.
                            unsafe { sistema.run(cell) };
                        });
                    }
                });
            }
            aplicar_diferidos(etapa, world);
        }
    }

    fn exigir_inicializado(&self) {
        assert!(
            self.inicializado,
            "schedule nao inicializado; chame initialize apos o ultimo add_system"
        );
    }
}

/// Aplica os command buffers da etapa, em ordem de insercao.
///
/// Sequencial de proposito: diferente dos sistemas de uma subetapa, comandos
/// **podem** se influenciar — um pode destruir a entidade que outro ia alterar.
/// Ordem fixa e o que torna o resultado reproduzivel.
fn aplicar_diferidos(etapa: &mut Etapa, world: &mut World) {
    for sistema in &mut etapa.sistemas {
        sistema.apply_deferred(world);
    }
}

impl std::fmt::Debug for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("Schedule");
        for etapa in &self.etapas {
            d.field(
                etapa.nome,
                &format_args!(
                    "{} sistemas em {} subetapas",
                    etapa.sistemas.len(),
                    etapa.subetapas.len()
                ),
            );
        }
        d.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{Res, ResMut};
    use crate::{Commands, Entity, Query};

    #[derive(Debug, PartialEq)]
    struct Posicao(f32);
    #[derive(Debug, PartialEq)]
    struct Velocidade(f32);
    #[derive(Debug, PartialEq)]
    struct Saude(i32);
    #[derive(Debug, PartialEq)]
    struct Quadros(u32);
    #[derive(Debug, PartialEq)]
    struct Passo(f32);
    struct Morta;

    fn mover(mut q: Query<(&mut Posicao, &Velocidade)>) {
        for (p, v) in q.iter() {
            p.0 += v.0;
        }
    }

    fn contar_quadros(mut n: ResMut<Quadros>) {
        n.0 += 1;
    }

    fn ler_passo(_p: Res<Passo>) {}

    fn danificar(mut q: Query<&mut Saude>) {
        for s in q.iter() {
            s.0 -= 1;
        }
    }

    fn mundo_base() -> World {
        let mut w = World::new();
        w.insert_resource(Quadros(0));
        w.insert_resource(Passo(0.5));
        w
    }

    #[test]
    fn etapas_saem_na_ordem_declarada() {
        let mut s = Schedule::new();
        s.add_stage("input").add_stage("simulacao").add_stage("render");
        assert_eq!(s.stages(), vec!["input", "simulacao", "render"]);
    }

    #[test]
    #[should_panic(expected = "ja existe")]
    fn etapa_duplicada_falha_alto() {
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_stage("simulacao");
    }

    #[test]
    #[should_panic(expected = "nao existe")]
    fn sistema_em_etapa_inexistente_falha_alto() {
        let mut s = Schedule::new();
        s.add_system("fantasma", mover);
    }

    #[test]
    fn sistemas_sem_conflito_ficam_na_mesma_subetapa() {
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.add_system("simulacao", contar_quadros);
        s.add_system("simulacao", danificar);
        s.initialize(&mut w);

        let subetapas = s.batches("simulacao").unwrap();
        assert_eq!(subetapas.len(), 1, "nenhum dos tres toca no que os outros tocam");
        assert_eq!(subetapas[0], vec![0, 1, 2]);
    }

    #[test]
    fn sistemas_em_conflito_ficam_em_subetapas_sucessivas() {
        // Os dois escrevem Posicao.
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.add_system("simulacao", mover);
        s.initialize(&mut w);

        let subetapas = s.batches("simulacao").unwrap();
        assert_eq!(subetapas.len(), 2);
        assert_eq!(subetapas[0], vec![0]);
        assert_eq!(subetapas[1], vec![1]);
    }

    #[test]
    fn leitores_de_recurso_paralelizam_entre_si() {
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", ler_passo);
        s.add_system("simulacao", ler_passo);
        s.initialize(&mut w);

        assert_eq!(s.batches("simulacao").unwrap().len(), 1, "duas leituras nao colidem");
    }

    #[test]
    fn escritor_de_recurso_serializa_contra_leitor() {
        fn escrever_passo(mut p: ResMut<Passo>) {
            p.0 += 1.0;
        }

        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", ler_passo);
        s.add_system("simulacao", escrever_passo);
        s.initialize(&mut w);

        assert_eq!(s.batches("simulacao").unwrap().len(), 2);
    }

    #[test]
    fn sistema_tardio_nao_passa_na_frente_de_quem_conflita() {
        // 0 escreve Posicao; 1 escreve Posicao (conflita com 0); 2 nao conflita
        // com ninguem. O guloso poria 2 junto de 0; aqui 2 tambem pode ir para a
        // subetapa 0, porque nao conflita com 0 nem com 1.
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.add_system("simulacao", mover);
        s.add_system("simulacao", danificar);
        s.initialize(&mut w);

        let subetapas = s.batches("simulacao").unwrap();
        assert_eq!(subetapas.len(), 2);
        assert_eq!(subetapas[0], vec![0, 2]);
        assert_eq!(subetapas[1], vec![1]);
    }

    #[test]
    fn execucao_sequencial_produz_o_efeito_esperado() {
        let mut w = mundo_base();
        let e = w.spawn((Posicao(0.0), Velocidade(2.0)));

        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.add_system("simulacao", contar_quadros);
        s.initialize(&mut w);

        s.run(&mut w);
        s.run(&mut w);

        assert_eq!(w.get::<Posicao>(e), Some(&Posicao(4.0)));
        assert_eq!(w.resource::<Quadros>(), &Quadros(2));
    }

    #[test]
    fn execucao_paralela_produz_o_mesmo_que_a_sequencial() {
        let montar = || {
            let mut w = mundo_base();
            for i in 0..500 {
                w.spawn((Posicao(i as f32), Velocidade(1.0), Saude(100)));
            }
            let mut s = Schedule::new();
            s.add_stage("simulacao");
            s.add_system("simulacao", mover);
            s.add_system("simulacao", danificar);
            s.add_system("simulacao", contar_quadros);
            s.initialize(&mut w);
            (w, s)
        };

        let (mut w1, mut s1) = montar();
        for _ in 0..10 {
            s1.run(&mut w1);
        }

        let (mut w2, mut s2) = montar();
        let pool = JobPool::new(4);
        for _ in 0..10 {
            s2.run_parallel(&mut w2, &pool);
        }

        let mut a: Vec<_> =
            w1.query::<(&Posicao, &Saude)>().map(|(p, s)| (p.0.to_bits(), s.0)).collect();
        let mut b: Vec<_> =
            w2.query::<(&Posicao, &Saude)>().map(|(p, s)| (p.0.to_bits(), s.0)).collect();
        a.sort_unstable();
        b.sort_unstable();

        assert_eq!(a, b);
        assert_eq!(w1.resource::<Quadros>(), w2.resource::<Quadros>());
    }

    #[test]
    fn etapas_sao_fronteiras_de_efeito() {
        // O sistema da segunda etapa precisa ver o que a primeira comandou.
        fn nascer(cmds: &mut Commands) {
            cmds.spawn((Posicao(1.0),));
        }
        fn contar_entidades(q: Query<Entity>, mut n: ResMut<Quadros>) {
            n.0 = u32::try_from(q.count()).unwrap();
        }

        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("criacao").add_stage("contagem");
        s.add_system("criacao", nascer);
        s.add_system("contagem", contar_entidades);
        s.initialize(&mut w);

        s.run(&mut w);

        assert_eq!(w.resource::<Quadros>().0, 1, "a etapa seguinte ve a entidade criada");
    }

    #[test]
    fn comandos_sao_aplicados_ao_fim_da_etapa() {
        fn marcar_mortos(q: Query<(Entity, &Saude)>, cmds: &mut Commands) {
            let mut q = q;
            for (e, s) in q.iter() {
                if s.0 <= 0 {
                    cmds.insert(e, (Morta,));
                }
            }
        }

        let mut w = mundo_base();
        w.spawn((Saude(0),));
        w.spawn((Saude(5),));

        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", marcar_mortos);
        s.initialize(&mut w);

        s.run(&mut w);

        assert_eq!(w.query_filtered::<&Saude, crate::With<Morta>>().count(), 1);
    }

    #[test]
    #[should_panic(expected = "nao inicializado")]
    fn rodar_sem_inicializar_falha_alto() {
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.run(&mut w);
    }

    #[test]
    fn batch_names_descreve_o_agrupamento() {
        let mut w = mundo_base();
        let mut s = Schedule::new();
        s.add_stage("simulacao");
        s.add_system("simulacao", mover);
        s.add_system("simulacao", danificar);
        s.initialize(&mut w);

        let nomes = s.batch_names("simulacao").unwrap();
        assert_eq!(nomes.len(), 1);
        assert_eq!(nomes[0].len(), 2);
        assert!(nomes[0][0].contains("mover"));
    }

    #[test]
    fn passos_sucessivos_sao_deterministicos() {
        let executar = |paralelo: bool| {
            let mut w = mundo_base();
            for i in 0..200 {
                w.spawn((Posicao(i as f32), Velocidade(0.5), Saude(10)));
            }
            let mut s = Schedule::new();
            s.add_stage("simulacao");
            s.add_system("simulacao", mover);
            s.add_system("simulacao", danificar);
            s.initialize(&mut w);

            let pool = JobPool::new(3);
            for _ in 0..20 {
                if paralelo {
                    s.run_parallel(&mut w, &pool);
                } else {
                    s.run(&mut w);
                }
            }

            let mut estado: Vec<_> =
                w.query::<(&Posicao, &Saude)>().map(|(p, h)| (p.0.to_bits(), h.0)).collect();
            estado.sort_unstable();
            estado
        };

        assert_eq!(executar(false), executar(false));
        assert_eq!(executar(true), executar(true));
        assert_eq!(executar(false), executar(true));
    }
}
