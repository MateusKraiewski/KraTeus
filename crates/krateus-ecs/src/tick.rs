//! Marcas de tempo logico para deteccao de mudanca.
//!
//! Cada instancia de componente carrega dois ticks: quando foi inserida e
//! quando foi exposta para escrita pela ultima vez. Cada sistema carrega o tick
//! em que rodou por ultimo. Comparar os dois responde "isto mudou desde a ultima
//! vez que eu olhei?" sem varrer nada.
//!
//! Ver a decisao D10 em `docs/DECISOES.md`.
//!
//! # O contrato que precisa estar visivel
//!
//! `&mut T` marca alterado **mesmo sem alteracao real**. Detectar mutacao de
//! fato exigiria comparar o valor antes e depois, o que custa mais do que o
//! filtro economiza. O contrato e "foi exposto para escrita", nao "mudou de
//! valor".
//!
//! # Por que a comparacao nao e `>`
//!
//! O contador e `u32` e da a volta. Comparar dois ticks diretamente inverteria o
//! resultado na virada. A comparacao correta mede **idade relativa ao tick
//! atual**, com subtracao circular: quem tem menos idade e mais novo.
//!
//! Isso funciona enquanto nenhum tick guardado for mais velho que
//! [`Tick::IDADE_MAXIMA`]. Um componente que nunca e tocado acabaria violando
//! isso, e e para esse caso que existe [`Tick::saneia`].

/// Instante logico, contado em execucoes de sistema.
///
/// Nao tem relacao com tempo de relogio: avanca uma vez por sistema executado.
/// Um passo de simulacao com N sistemas avanca N ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Tick(u32);

impl Tick {
    /// Tick inicial de um mundo recem-criado.
    pub const ZERO: Self = Self(0);

    /// Maior idade que um tick guardado pode ter antes de a comparacao
    /// circular deixar de distinguir "velho" de "novo".
    ///
    /// Metade do espaco de `u32`: acima disso, a subtracao circular nao
    /// consegue mais dizer de que lado da virada o valor esta.
    pub const IDADE_MAXIMA: u32 = u32::MAX / 2;

    /// Quantos ticks podem passar entre duas varreduras de saneamento.
    ///
    /// Depois de uma varredura, toda idade guardada e no maximo
    /// [`IDADE_MAXIMA`](Self::IDADE_MAXIMA). Ate a proxima, elas crescem no
    /// maximo mais este tanto — e a soma precisa caber em `u32`, senao a
    /// subtracao circular perde o sentido.
    ///
    /// A ordem de grandeza torna a varredura rara: a 60 passos por segundo com
    /// 60 sistemas, sao dias de execucao continua entre uma e outra.
    pub const INTERVALO_DE_SANEAMENTO: u32 = u32::MAX / 4;

    /// Constroi um tick a partir do valor bruto.
    #[inline]
    #[must_use]
    pub const fn new(valor: u32) -> Self {
        Self(valor)
    }

    /// Valor bruto.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Avanca `n` ticks, dando a volta se preciso.
    #[inline]
    #[must_use]
    pub const fn adiantado(self, n: u32) -> Self {
        Self(self.0.wrapping_add(n))
    }

    /// Idade deste tick em relacao a `agora`, saturada em
    /// [`IDADE_MAXIMA`](Self::IDADE_MAXIMA).
    #[inline]
    #[must_use]
    pub const fn idade(self, agora: Self) -> u32 {
        let bruta = agora.0.wrapping_sub(self.0);
        if bruta > Self::IDADE_MAXIMA { Self::IDADE_MAXIMA } else { bruta }
    }

    /// Indica se `self` e mais recente que `outro`, do ponto de vista de
    /// `agora`.
    ///
    /// Menos idade significa mais novo. Dois ticks igualmente velhos — ambos
    /// saturados — nao sao considerados mais novos um que o outro; e a
    /// degradacao que a varredura de saneamento existe para evitar.
    #[inline]
    #[must_use]
    pub const fn is_newer_than(self, outro: Self, agora: Self) -> bool {
        outro.idade(agora) > self.idade(agora)
    }

    /// Puxa o tick para a idade maxima, se ele estiver mais velho que isso.
    ///
    /// Devolve `true` se houve ajuste. E o unico jeito de impedir que um
    /// componente esquecido por muito tempo comece a parecer recente depois da
    /// virada do contador.
    #[inline]
    pub const fn saneia(&mut self, agora: Self) -> bool {
        if agora.0.wrapping_sub(self.0) > Self::IDADE_MAXIMA {
            self.0 = agora.0.wrapping_sub(Self::IDADE_MAXIMA);
            true
        } else {
            false
        }
    }
}

/// Quando uma instancia de componente foi inserida e alterada.
///
/// Fica em array paralelo a coluna, e nao intercalado com os dados: um sistema
/// que nao filtra por mudanca nao deve arrastar 8 bytes por componente para o
/// cache sem usar (secao 11 do documento de visao, e a D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentTicks {
    /// Tick em que o componente passou a existir nesta entidade.
    pub added: Tick,
    /// Tick em que o componente foi exposto para escrita pela ultima vez.
    pub changed: Tick,
}

impl ComponentTicks {
    /// Ticks de um componente recem-inserido.
    #[inline]
    #[must_use]
    pub const fn novo(tick: Tick) -> Self {
        Self { added: tick, changed: tick }
    }

    /// Indica se o componente foi inserido depois de `last_run`.
    #[inline]
    #[must_use]
    pub const fn is_added(&self, last_run: Tick, agora: Tick) -> bool {
        self.added.is_newer_than(last_run, agora)
    }

    /// Indica se o componente foi exposto para escrita depois de `last_run`.
    ///
    /// Inserir tambem conta como alterar, entao `is_added` implica
    /// `is_changed`.
    #[inline]
    #[must_use]
    pub const fn is_changed(&self, last_run: Tick, agora: Tick) -> bool {
        self.changed.is_newer_than(last_run, agora)
    }

    /// Aplica a varredura de saneamento aos dois ticks.
    pub const fn saneia(&mut self, agora: Tick) -> bool {
        // Sem curto-circuito: os dois precisam ser visitados.
        let a = self.added.saneia(agora);
        let c = self.changed.saneia(agora);
        a || c
    }
}

/// Os dois ticks que um sistema precisa para decidir o que mudou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemTicks {
    /// Tick em que este sistema rodou pela ultima vez.
    pub last_run: Tick,
    /// Tick desta execucao. E o que sera gravado no que for escrito.
    pub atual: Tick,
}

impl SystemTicks {
    /// Constroi o par.
    #[inline]
    #[must_use]
    pub const fn new(last_run: Tick, atual: Tick) -> Self {
        Self { last_run, atual }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_mais_recente_vence() {
        let agora = Tick::new(100);
        assert!(Tick::new(90).is_newer_than(Tick::new(50), agora));
        assert!(!Tick::new(50).is_newer_than(Tick::new(90), agora));
    }

    #[test]
    fn ticks_iguais_nao_sao_mais_novos_um_que_o_outro() {
        let agora = Tick::new(100);
        assert!(!Tick::new(50).is_newer_than(Tick::new(50), agora));
    }

    #[test]
    fn comparacao_sobrevive_a_virada_do_contador() {
        // `agora` deu a volta; o tick anterior ficou no fim do espaco de u32.
        let agora = Tick::new(10);
        let antes_da_virada = Tick::new(u32::MAX - 5);
        let bem_antes = Tick::new(u32::MAX - 1_000);

        assert_eq!(antes_da_virada.idade(agora), 16);
        assert!(antes_da_virada.is_newer_than(bem_antes, agora));
        assert!(!bem_antes.is_newer_than(antes_da_virada, agora));
    }

    #[test]
    fn idade_satura_na_idade_maxima() {
        let agora = Tick::new(0);
        let antiquissimo = Tick::new(1);
        assert_eq!(antiquissimo.idade(agora), Tick::IDADE_MAXIMA);
    }

    #[test]
    fn sem_saneamento_um_tick_muito_velho_confunde_a_comparacao() {
        // Documenta o problema que a varredura resolve: dois ticks de idades
        // diferentes, ambas acima do teto, ficam indistinguiveis.
        let agora = Tick::new(0);
        let velho = Tick::new(1);
        let mais_velho = Tick::new(1000);

        assert_eq!(velho.idade(agora), mais_velho.idade(agora));
        assert!(!velho.is_newer_than(mais_velho, agora), "a distincao se perde");
    }

    #[test]
    fn saneamento_puxa_o_tick_para_a_idade_maxima() {
        let agora = Tick::new(1_000_000);
        let mut esquecido = Tick::new(agora.get().wrapping_sub(Tick::IDADE_MAXIMA + 500));

        assert!(esquecido.saneia(agora));
        assert_eq!(esquecido.idade(agora), Tick::IDADE_MAXIMA);
    }

    #[test]
    fn saneamento_nao_mexe_em_tick_recente() {
        let agora = Tick::new(1000);
        let mut recente = Tick::new(990);

        assert!(!recente.saneia(agora));
        assert_eq!(recente, Tick::new(990));
    }

    #[test]
    fn intervalo_de_saneamento_cabe_no_espaco_restante() {
        // A soma precisa caber em u32, senao a subtracao circular perde o
        // sentido antes da proxima varredura.
        let sobra = u32::MAX - Tick::IDADE_MAXIMA;
        assert!(
            Tick::INTERVALO_DE_SANEAMENTO < sobra,
            "intervalo {} nao cabe na sobra {sobra}",
            Tick::INTERVALO_DE_SANEAMENTO
        );
    }

    #[test]
    fn componente_recem_inserido_conta_como_adicionado_e_alterado() {
        let t = ComponentTicks::novo(Tick::new(10));
        let agora = Tick::new(10);
        let last_run = Tick::new(5);

        assert!(t.is_added(last_run, agora));
        assert!(t.is_changed(last_run, agora), "inserir tambem e alterar");
    }

    #[test]
    fn componente_antigo_alterado_conta_so_como_alterado() {
        let mut t = ComponentTicks::novo(Tick::new(1));
        t.changed = Tick::new(10);

        let agora = Tick::new(10);
        let last_run = Tick::new(5);

        assert!(!t.is_added(last_run, agora));
        assert!(t.is_changed(last_run, agora));
    }

    #[test]
    fn componente_intocado_nao_conta_como_alterado() {
        let t = ComponentTicks::novo(Tick::new(1));
        assert!(!t.is_changed(Tick::new(5), Tick::new(10)));
    }

    #[test]
    fn saneamento_visita_os_dois_ticks() {
        let agora = Tick::new(1_000_000);
        let velho = Tick::new(agora.get().wrapping_sub(Tick::IDADE_MAXIMA + 500));
        let mut t = ComponentTicks { added: velho, changed: velho };

        assert!(t.saneia(agora));
        assert_eq!(t.added.idade(agora), Tick::IDADE_MAXIMA);
        assert_eq!(t.changed.idade(agora), Tick::IDADE_MAXIMA);
    }

    #[test]
    fn saneamento_visita_o_segundo_mesmo_quando_o_primeiro_nao_muda() {
        let agora = Tick::new(1_000_000);
        let mut t = ComponentTicks {
            added: Tick::new(agora.get() - 10),
            changed: Tick::new(agora.get().wrapping_sub(Tick::IDADE_MAXIMA + 500)),
        };

        assert!(t.saneia(agora));
        assert_eq!(t.changed.idade(agora), Tick::IDADE_MAXIMA, "o segundo precisa ser visitado");
    }
}
