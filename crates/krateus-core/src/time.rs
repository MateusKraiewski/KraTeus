//! Relogio de timestep fixo.
//!
//! A simulacao avanca em passos de duracao constante, independentemente do
//! tempo real gasto por frame. Essa e a base dos requisitos das secoes 15 e 16
//! do documento de visao: replay, save/load e networking deterministico so sao
//! possiveis se o passo de simulacao nao depender da taxa de quadros da
//! maquina.
//!
//! O padrao de uso separa acumular tempo real de consumir passos fixos:
//!
//! ```
//! use std::time::Duration;
//! use krateus_core::time::Clock;
//!
//! let mut clock = Clock::new(60);
//! clock.accumulate(Duration::from_millis(50));
//!
//! let mut passos = 0;
//! while let Some(_time) = clock.next_tick() {
//!     passos += 1;
//! }
//!
//! assert_eq!(passos, 3); // 50 ms cabem tres passos de 16,66 ms
//! let _alpha = clock.alpha(); // sobra, para interpolar o render
//! ```

use std::time::Duration;

/// Estado temporal de um passo de simulacao.
///
/// Todos os campos sao derivados da contagem de passos, nunca do relogio do
/// sistema: a mesma sequencia de passos produz os mesmos valores em qualquer
/// maquina.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Time {
    /// Indice do passo, comecando em 1.
    pub tick: u64,
    /// Duracao fixa do passo.
    pub delta: Duration,
    /// Tempo simulado acumulado ao fim deste passo.
    pub elapsed: Duration,
}

impl Time {
    /// Duracao do passo em segundos.
    #[inline]
    #[must_use]
    pub fn delta_seconds(&self) -> f32 {
        self.delta.as_secs_f32()
    }
}

/// Acumulador de timestep fixo com protecao contra espiral de morte.
#[derive(Debug, Clone)]
pub struct Clock {
    timestep: Duration,
    acumulado: Duration,
    elapsed: Duration,
    tick: u64,
    max_passos_por_frame: u32,
}

impl Clock {
    /// Passos maximos consumidos por frame antes de descartar o excedente.
    ///
    /// Sem esse teto, um frame lento gera passos extras, que atrasam o frame
    /// seguinte, que gera mais passos ainda: a simulacao nunca alcanca o tempo
    /// real. Descartar tempo degrada a simulacao de forma visivel, o que e
    /// preferivel a travar.
    pub const MAX_PASSOS_POR_FRAME: u32 = 8;

    /// Cria um relogio com a frequencia de simulacao dada, em passos por
    /// segundo.
    ///
    /// # Panics
    ///
    /// Se `ticks_per_second` for zero.
    #[must_use]
    pub fn new(ticks_per_second: u32) -> Self {
        assert!(ticks_per_second > 0, "a frequencia de simulacao precisa ser positiva");
        Self {
            timestep: Duration::from_nanos(1_000_000_000 / u64::from(ticks_per_second)),
            acumulado: Duration::ZERO,
            elapsed: Duration::ZERO,
            tick: 0,
            max_passos_por_frame: Self::MAX_PASSOS_POR_FRAME,
        }
    }

    /// Substitui o teto de passos por frame.
    #[must_use]
    pub fn with_max_steps_per_frame(mut self, max: u32) -> Self {
        assert!(max > 0, "o teto de passos precisa ser positivo");
        self.max_passos_por_frame = max;
        self
    }

    /// Duracao fixa de cada passo.
    #[inline]
    #[must_use]
    pub const fn timestep(&self) -> Duration {
        self.timestep
    }

    /// Passos ja executados.
    #[inline]
    #[must_use]
    pub const fn tick_count(&self) -> u64 {
        self.tick
    }

    /// Tempo simulado acumulado.
    #[inline]
    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Adiciona tempo real medido, saturando no teto de passos por frame.
    ///
    /// Devolve `true` se houve descarte — sinal util para o profiler da Fase 7.
    pub fn accumulate(&mut self, real_delta: Duration) -> bool {
        self.acumulado = self.acumulado.saturating_add(real_delta);

        let teto = self.timestep * self.max_passos_por_frame;
        if self.acumulado > teto {
            self.acumulado = teto;
            return true;
        }
        false
    }

    /// Consome um passo fixo, se houver tempo acumulado suficiente.
    ///
    /// Chamar em laco ate devolver `None`.
    pub fn next_tick(&mut self) -> Option<Time> {
        if self.acumulado < self.timestep {
            return None;
        }

        self.acumulado -= self.timestep;
        self.tick += 1;
        self.elapsed += self.timestep;

        Some(Time { tick: self.tick, delta: self.timestep, elapsed: self.elapsed })
    }

    /// Fracao do passo seguinte ja acumulada, no intervalo `[0, 1)`.
    ///
    /// E o fator de interpolacao que o renderer usa para desenhar entre dois
    /// estados de simulacao, evitando o tremor visual de renderizar sempre o
    /// ultimo passo fechado.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        self.acumulado.as_secs_f32() / self.timestep.as_secs_f32()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consome_passos_exatos() {
        let mut clock = Clock::new(60);
        clock.accumulate(Duration::from_millis(50));

        let passos: Vec<_> = std::iter::from_fn(|| clock.next_tick()).collect();
        assert_eq!(passos.len(), 3);
        assert_eq!(passos[2].tick, 3);
    }

    #[test]
    fn tempo_decorrido_nao_deriva() {
        let mut clock = Clock::new(64);

        // Alimentado em fatias irregulares, como um frame real.
        for ms in [7, 3, 11, 5, 9, 1, 20, 4] {
            clock.accumulate(Duration::from_millis(ms));
            while clock.next_tick().is_some() {}
        }

        // O tempo simulado e sempre um multiplo exato do passo, qualquer que
        // tenha sido o padrao de chegada do tempo real.
        assert_eq!(clock.elapsed(), clock.timestep() * u32::try_from(clock.tick_count()).unwrap());
    }

    #[test]
    fn frame_longo_nao_gera_espiral() {
        let mut clock = Clock::new(60);
        let descartou = clock.accumulate(Duration::from_secs(10));
        assert!(descartou);

        let passos = std::iter::from_fn(|| clock.next_tick()).count();
        assert_eq!(passos as u32, Clock::MAX_PASSOS_POR_FRAME);
    }

    #[test]
    fn alpha_fica_no_intervalo_unitario() {
        let mut clock = Clock::new(60);
        clock.accumulate(Duration::from_micros(25_000));
        while clock.next_tick().is_some() {}

        let alpha = clock.alpha();
        assert!((0.0..1.0).contains(&alpha), "alpha fora do intervalo: {alpha}");
    }

    #[test]
    fn mesma_entrada_produz_mesma_sequencia() {
        let executar = || {
            let mut clock = Clock::new(60);
            let mut hash = 0_u64;
            for ms in [16, 17, 16, 33, 8, 16] {
                clock.accumulate(Duration::from_millis(ms));
                while let Some(t) = clock.next_tick() {
                    hash = hash.wrapping_mul(31).wrapping_add(t.elapsed.as_nanos() as u64);
                }
            }
            hash
        };

        assert_eq!(executar(), executar());
    }
}
