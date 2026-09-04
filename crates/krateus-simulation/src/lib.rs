//! Laco de simulacao: liga o relogio de timestep fixo ao scheduler.
//!
//! O relogio ([`Clock`]) e o scheduler ([`Schedule`]) existem separados de
//! proposito — um mede tempo, o outro organiza trabalho. Esta crate e a costura:
//! consome tempo real, decide quantos passos fixos cabem nele, e roda o
//! scheduler uma vez por passo.
//!
//! ```
//! use std::time::Duration;
//! use krateus_ecs::{Query, Res, Schedule, World};
//! use krateus_simulation::{SimulationLoop, Time};
//!
//! struct Posicao(f32);
//! struct Velocidade(f32);
//!
//! fn integrar(mut q: Query<(&mut Posicao, &Velocidade)>, tempo: Res<Time>) {
//!     for (p, v) in q.iter() {
//!         p.0 += v.0 * tempo.delta_seconds();
//!     }
//! }
//!
//! let mut world = World::new();
//! world.spawn((Posicao(0.0), Velocidade(60.0)));
//!
//! let mut schedule = Schedule::new();
//! schedule.add_stage("simulacao");
//! schedule.add_system("simulacao", integrar);
//!
//! let mut laco = SimulationLoop::new(60);
//! laco.initialize(&mut world, &mut schedule);
//!
//! // Um segundo de tempo real, entregue quadro a quadro. Cada fatia fica
//! // abaixo do teto de passos por quadro, entao nada e descartado.
//! for _ in 0..10 {
//!     laco.advance(Duration::from_millis(100), &mut world, &mut schedule);
//! }
//!
//! // 60 passos de 1/60 s a 60 unidades por segundo: a posicao anda ~60.
//! let x = world.query::<&Posicao>().next().unwrap().0;
//! assert!((x - 60.0).abs() < 0.1, "posicao foi {x}");
//! ```
//!
//! # Por que o passo e fixo
//!
//! Amarrar a simulacao a taxa de quadros faz o resultado depender da maquina: a
//! mesma partida diverge entre um computador rapido e um lento. Save, replay e
//! networking em lockstep dependem de que nao seja assim (secao 15 do documento
//! de visao, e a D09 em `docs/DECISOES.md`).
//!
//! O preco e que o tempo real quase nunca cabe num numero inteiro de passos. A
//! sobra fica acumulada para o proximo quadro, e e exposta como
//! [`alpha`](SimulationLoop::alpha) para o renderer interpolar entre o ultimo
//! estado fechado e o proximo — sem isso, o movimento treme.

use std::time::Duration;

use krateus_core::jobs::JobPool;
use krateus_core::time::Clock;
use krateus_ecs::{Schedule, World};

pub use krateus_core::time::Time;

/// O que aconteceu num avanco do laco.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Avanco {
    /// Passos fixos executados neste avanco.
    pub passos: u32,
    /// Fracao do proximo passo ja acumulada, em `[0, 1)`.
    ///
    /// E o fator de interpolacao do renderer.
    pub alpha: f32,
    /// Se houve tempo real descartado por exceder o teto de passos por quadro.
    ///
    /// Verdadeiro significa que a simulacao ficou para tras do tempo real — um
    /// sinal para o profiler da Fase 7, nao um erro.
    pub descartou_tempo: bool,
}

/// Laco de simulacao com timestep fixo.
///
/// Nao possui o mundo nem o scheduler: recebe os dois a cada avanco. Isso deixa
/// o editor, os testes e o runtime conduzirem o laco de formas diferentes sem
/// que esta crate precise saber de qual se trata.
#[derive(Debug, Clone)]
pub struct SimulationLoop {
    clock: Clock,
}

impl SimulationLoop {
    /// Cria o laco com a frequencia de simulacao dada, em passos por segundo.
    ///
    /// # Panics
    ///
    /// Se `ticks_per_second` for zero.
    #[must_use]
    pub fn new(ticks_per_second: u32) -> Self {
        Self { clock: Clock::new(ticks_per_second) }
    }

    /// Substitui o teto de passos executados por avanco.
    ///
    /// Ver [`Clock::with_max_steps_per_frame`].
    #[must_use]
    pub fn with_max_steps_per_frame(mut self, max: u32) -> Self {
        self.clock = self.clock.with_max_steps_per_frame(max);
        self
    }

    /// Duracao fixa de cada passo.
    #[inline]
    #[must_use]
    pub const fn timestep(&self) -> Duration {
        self.clock.timestep()
    }

    /// Passos executados desde a criacao do laco.
    #[inline]
    #[must_use]
    pub const fn tick_count(&self) -> u64 {
        self.clock.tick_count()
    }

    /// Tempo simulado acumulado.
    #[inline]
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }

    /// Fracao do proximo passo ja acumulada, em `[0, 1)`.
    #[inline]
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.clock.alpha()
    }

    /// Prepara mundo e scheduler para rodar.
    ///
    /// Insere o recurso [`Time`], para que os sistemas possam pedir `Res<Time>`
    /// ja na inicializacao, e resolve os parametros de todos os sistemas.
    pub fn initialize(&self, world: &mut World, schedule: &mut Schedule) {
        self.publicar_tempo(world, self.tempo_inicial());
        schedule.initialize(world);
    }

    /// Consome `real_delta` de tempo real e roda os passos fixos que couberem,
    /// em uma unica thread.
    pub fn advance(
        &mut self,
        real_delta: Duration,
        world: &mut World,
        schedule: &mut Schedule,
    ) -> Avanco {
        self.avancar(real_delta, world, |world| schedule.run(world))
    }

    /// Como [`advance`](Self::advance), paralelizando dentro de cada subetapa.
    pub fn advance_parallel(
        &mut self,
        real_delta: Duration,
        world: &mut World,
        schedule: &mut Schedule,
        pool: &JobPool,
    ) -> Avanco {
        self.avancar(real_delta, world, |world| schedule.run_parallel(world, pool))
    }

    fn avancar(
        &mut self,
        real_delta: Duration,
        world: &mut World,
        mut rodar: impl FnMut(&mut World),
    ) -> Avanco {
        let descartou_tempo = self.clock.accumulate(real_delta);
        let mut passos = 0;

        while let Some(tempo) = self.clock.next_tick() {
            self.publicar_tempo(world, tempo);
            rodar(world);
            passos += 1;
        }

        Avanco { passos, alpha: self.clock.alpha(), descartou_tempo }
    }

    /// Estado temporal antes do primeiro passo.
    fn tempo_inicial(&self) -> Time {
        Time { tick: 0, delta: self.clock.timestep(), elapsed: Duration::ZERO }
    }

    /// Atualiza o recurso `Time` no lugar.
    ///
    /// Sobrescrever o valor existente em vez de reinserir evita uma alocacao por
    /// passo — a secao 11 do documento de visao pede explicitamente que nao se
    /// aloque por quadro.
    fn publicar_tempo(&self, world: &mut World, tempo: Time) {
        if let Some(slot) = world.get_resource_mut::<Time>() {
            *slot = tempo;
        } else {
            world.insert_resource(tempo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use krateus_ecs::{Query, Res, ResMut};

    #[derive(Debug, PartialEq)]
    struct Posicao(f32);
    #[derive(Debug, PartialEq)]
    struct Velocidade(f32);
    #[derive(Debug, PartialEq)]
    struct Passos(u32);
    #[derive(Debug, PartialEq)]
    struct UltimoTick(u64);

    fn integrar(mut q: Query<(&mut Posicao, &Velocidade)>, tempo: Res<Time>) {
        for (p, v) in q.iter() {
            p.0 += v.0 * tempo.delta_seconds();
        }
    }

    fn contar_passos(mut n: ResMut<Passos>) {
        n.0 += 1;
    }

    fn registrar_tick(tempo: Res<Time>, mut ultimo: ResMut<UltimoTick>) {
        ultimo.0 = tempo.tick;
    }

    fn montar() -> (World, Schedule) {
        let mut world = World::new();
        world.insert_resource(Passos(0));
        world.insert_resource(UltimoTick(0));

        let mut schedule = Schedule::new();
        schedule.add_stage("simulacao");
        schedule.add_system("simulacao", integrar);
        schedule.add_system("simulacao", contar_passos);
        schedule.add_system("simulacao", registrar_tick);
        (world, schedule)
    }

    #[test]
    fn tempo_insuficiente_nao_roda_passo() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        let avanco = laco.advance(Duration::from_millis(5), &mut w, &mut s);

        assert_eq!(avanco.passos, 0);
        assert_eq!(w.resource::<Passos>(), &Passos(0));
        assert!(avanco.alpha > 0.0, "a sobra fica acumulada");
    }

    #[test]
    fn um_segundo_a_60hz_produz_60_passos() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        let mut total = 0;
        for _ in 0..10 {
            total += laco.advance(Duration::from_millis(100), &mut w, &mut s).passos;
        }

        assert_eq!(total, 60);
        assert_eq!(w.resource::<Passos>(), &Passos(60));
        assert_eq!(laco.tick_count(), 60);
    }

    #[test]
    fn quadro_irregular_nao_altera_o_tempo_simulado() {
        // O ponto do timestep fixo: a chegada do tempo real nao influencia o
        // resultado, so quantos passos rodam em cada avanco.
        //
        // A propriedade vale enquanto nada for descartado. O teto de passos por
        // quadro se aplica ao acumulador, nao a fatia: um quadro grande o
        // bastante para estourar o teto joga tempo fora de proposito, e ai o
        // total de passos muda mesmo com o mesmo tempo real. Por isso o teste
        // confere que nao houve descarte.
        let executar = |fatias: &[u64]| {
            let (mut w, mut s) = montar();
            let mut laco = SimulationLoop::new(60);
            laco.initialize(&mut w, &mut s);

            for &ms in fatias {
                let avanco = laco.advance(Duration::from_millis(ms), &mut w, &mut s);
                assert!(!avanco.descartou_tempo, "fatia de {ms} ms estourou o teto");
            }
            (laco.tick_count(), w.resource::<Passos>().0)
        };

        let regular = executar(&[100; 10]);
        let irregular = executar(&[7, 3, 90, 45, 12, 80, 33, 100, 100, 100, 100, 100, 90, 40, 100]);

        assert_eq!(regular.0 + regular.1 as u64, 120, "1000 ms a 60 Hz sao 60 passos");
        assert_eq!(regular, irregular, "o mesmo tempo real, venha como vier");
    }

    #[test]
    fn sistemas_recebem_o_tempo_do_passo() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        laco.advance(Duration::from_millis(100), &mut w, &mut s);

        assert_eq!(w.resource::<UltimoTick>(), &UltimoTick(6), "o ultimo passo foi o sexto");
        assert_eq!(w.resource::<Time>().tick, 6);
    }

    #[test]
    fn integracao_avanca_a_posicao_proporcional_ao_tempo() {
        let (mut w, mut s) = montar();
        let e = w.spawn((Posicao(0.0), Velocidade(60.0)));
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        for _ in 0..10 {
            laco.advance(Duration::from_millis(100), &mut w, &mut s);
        }

        // 60 passos de 1/60 s a 60 u/s: quase exatamente 60 unidades. O timestep
        // e 16_666_666 ns, nao 1/60 exato, entao ha uma folga pequena.
        let x = w.get::<Posicao>(e).unwrap().0;
        assert!((x - 60.0).abs() < 0.01, "posicao foi {x}");
    }

    #[test]
    fn quadro_muito_longo_descarta_tempo_em_vez_de_espiralar() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        let avanco = laco.advance(Duration::from_secs(10), &mut w, &mut s);

        assert!(avanco.descartou_tempo);
        assert_eq!(avanco.passos, 8, "o teto padrao do Clock");
    }

    #[test]
    fn alpha_fica_no_intervalo_unitario() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        for ms in [7, 13, 25, 4, 40] {
            let avanco = laco.advance(Duration::from_millis(ms), &mut w, &mut s);
            assert!(
                (0.0..1.0).contains(&avanco.alpha),
                "alpha fora do intervalo: {}",
                avanco.alpha
            );
            assert_eq!(avanco.alpha, laco.alpha());
        }
    }

    #[test]
    fn paralelo_produz_o_mesmo_que_sequencial() {
        let rodar = |paralelo: bool| {
            let (mut w, mut s) = montar();
            for i in 0..300 {
                w.spawn((Posicao(i as f32), Velocidade(1.0)));
            }
            let mut laco = SimulationLoop::new(60);
            laco.initialize(&mut w, &mut s);

            let pool = JobPool::new(3);
            for _ in 0..20 {
                if paralelo {
                    laco.advance_parallel(Duration::from_millis(50), &mut w, &mut s, &pool);
                } else {
                    laco.advance(Duration::from_millis(50), &mut w, &mut s);
                }
            }

            let mut estado: Vec<_> = w.query::<&Posicao>().map(|p| p.0.to_bits()).collect();
            estado.sort_unstable();
            (estado, w.resource::<Passos>().0, laco.tick_count())
        };

        assert_eq!(rodar(false), rodar(true));
    }

    #[test]
    fn tempo_e_publicado_antes_da_inicializacao() {
        // Um sistema que pede Res<Time> precisa achar o recurso ja em
        // `initialize`, senao o registro do acesso falharia na primeira execucao.
        let (mut w, mut s) = montar();
        let laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);

        assert!(w.contains_resource::<Time>());
        assert_eq!(w.resource::<Time>().tick, 0);
    }

    #[test]
    fn publicar_tempo_nao_recria_o_recurso() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60);
        laco.initialize(&mut w, &mut s);
        let registrados = w.resources().registered();

        for _ in 0..100 {
            laco.advance(Duration::from_millis(20), &mut w, &mut s);
        }

        assert_eq!(w.resources().registered(), registrados, "nenhum recurso novo por passo");
    }

    #[test]
    fn teto_de_passos_e_configuravel() {
        let (mut w, mut s) = montar();
        let mut laco = SimulationLoop::new(60).with_max_steps_per_frame(2);
        laco.initialize(&mut w, &mut s);

        let avanco = laco.advance(Duration::from_secs(1), &mut w, &mut s);
        assert_eq!(avanco.passos, 2);
        assert!(avanco.descartou_tempo);
    }
}
