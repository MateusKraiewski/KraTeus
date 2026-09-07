//! Balistica: resolver um salto e percorrer a trajetoria.
//!
//! E a ferramenta descrita no §6 do documento de visao. Dada a altura e a
//! distancia de um salto, ela devolve a velocidade inicial, o apice, os tempos
//! de subida e descida e o tempo total de voo — sem depender de animacao
//! pre-programada.
//!
//! ```
//! use krateus_physics::{Fidelidade, Salto, G_TERRA};
//!
//! // Um salto de 2 m de altura que cobre 6 m de distancia.
//! let salto = Salto { altura_apice: 2.0, distancia: 6.0, desnivel: 0.0 };
//! let l = salto.resolver(G_TERRA).expect("salto possivel");
//!
//! assert!((l.velocidade_vertical - 6.264).abs() < 0.01);
//! assert!((l.tempo_total - 1.277).abs() < 0.01);
//! assert!((l.altura_maxima - 2.0).abs() < 1e-5);
//!
//! // A analitica e exata em qualquer resolucao. A integrada acumula erro de
//! // primeira ordem: com 32 amostras ela ja erra mais de 10 cm no apice.
//! let a = l.amostrar(G_TERRA, Fidelidade::Analitica, 32);
//! let b = l.amostrar(G_TERRA, Fidelidade::Integrada, 32);
//! assert!((a[16].y - b[16].y).abs() > 0.1);
//!
//! // Refinar o passo aproxima as duas — e e por isso que a fidelidade e uma
//! // escolha, e nao um detalhe interno.
//! let a = l.amostrar(G_TERRA, Fidelidade::Analitica, 2048);
//! let b = l.amostrar(G_TERRA, Fidelidade::Integrada, 2048);
//! assert!((a[1024].y - b[1024].y).abs() < 0.005);
//! ```
//!
//! # Por que a solucao nao usa trigonometria
//!
//! A formulacao obvia passa por angulo: `tan(θ) = 4h/d`, depois `v0 = ...`. Ela
//! esta certa e e ruim aqui, porque `tan` e `atan` vem da libm do sistema — e e
//! justamente em funcoes transcendentais que plataformas divergem no ultimo bit
//! (D09 em `docs/DECISOES.md`).
//!
//! Resolver direto pelas componentes usa apenas raiz quadrada e aritmetica, que
//! o IEEE-754 define exatamente. O angulo continua disponivel em
//! [`Lancamento::angulo`], mas como conveniencia de apresentacao: ele nao
//! alimenta a simulacao.

use glam::Vec2;

/// Aceleracao da gravidade na Terra, em m/s².
pub const G_TERRA: f32 = 9.806_65;

/// O que se pede a um salto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Salto {
    /// Altura do apice acima do ponto de partida.
    pub altura_apice: f32,
    /// Distancia horizontal entre partida e chegada.
    pub distancia: f32,
    /// Altura da chegada em relacao a partida.
    ///
    /// Positivo sobe — pular para uma plataforma; negativo desce.
    pub desnivel: f32,
}

/// Por que um salto nao tem solucao.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SaltoImpossivel {
    /// Gravidade nula ou negativa: nao ha o que puxar o corpo de volta.
    #[error("a gravidade precisa ser positiva")]
    SemGravidade,
    /// Apice na altura da partida ou abaixo dela.
    #[error("o apice precisa ficar acima do ponto de partida")]
    ApiceNaoSobe,
    /// Apice abaixo do ponto de chegada: o corpo nunca alcancaria o alvo.
    #[error("o apice precisa ficar acima do ponto de chegada")]
    ApiceAbaixoDaChegada,
    /// Distancia horizontal negativa.
    #[error("a distancia nao pode ser negativa")]
    DistanciaNegativa,
}

/// O lancamento que realiza um salto.
///
/// As velocidades sao componentes, e nao modulo mais angulo, de proposito: e a
/// forma que o integrador consome e a que nao passa por trigonometria.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lancamento {
    /// Velocidade horizontal, constante durante o voo sem arrasto.
    pub velocidade_horizontal: f32,
    /// Velocidade vertical no instante da partida.
    pub velocidade_vertical: f32,
    /// Tempo da partida ate o apice.
    pub tempo_subida: f32,
    /// Tempo do apice ate a chegada.
    pub tempo_descida: f32,
    /// Tempo total de voo.
    pub tempo_total: f32,
    /// Altura do apice acima da partida. Repete o pedido, para que o resultado
    /// seja autocontido.
    pub altura_maxima: f32,
}

impl Salto {
    /// Resolve o lancamento que cumpre este salto.
    ///
    /// # Errors
    ///
    /// Quando o salto pedido nao existe — ver [`SaltoImpossivel`].
    pub fn resolver(&self, gravidade: f32) -> Result<Lancamento, SaltoImpossivel> {
        if gravidade <= 0.0 {
            return Err(SaltoImpossivel::SemGravidade);
        }
        if self.distancia < 0.0 {
            return Err(SaltoImpossivel::DistanciaNegativa);
        }
        if self.altura_apice <= 0.0 {
            return Err(SaltoImpossivel::ApiceNaoSobe);
        }
        if self.altura_apice <= self.desnivel {
            return Err(SaltoImpossivel::ApiceAbaixoDaChegada);
        }

        // Subida: a velocidade vertical que atinge exatamente o apice.
        //   v² = 2·g·h
        let velocidade_vertical = (2.0 * gravidade * self.altura_apice).sqrt();
        let tempo_subida = velocidade_vertical / gravidade;

        // Descida: queda livre do apice ate a altura de chegada.
        let queda = self.altura_apice - self.desnivel;
        let tempo_descida = (2.0 * queda / gravidade).sqrt();

        let tempo_total = tempo_subida + tempo_descida;

        // A horizontal e a unica componente que a distancia determina, e so
        // depois de o tempo de voo estar fechado.
        let velocidade_horizontal =
            if tempo_total > 0.0 { self.distancia / tempo_total } else { 0.0 };

        Ok(Lancamento {
            velocidade_horizontal,
            velocidade_vertical,
            tempo_subida,
            tempo_descida,
            tempo_total,
            altura_maxima: self.altura_apice,
        })
    }
}

/// Quanto custa percorrer a trajetoria.
///
/// A escolha existe para que o custo acompanhe o uso (§18): uma previsao de
/// mira precisa de poucos pontos exatos; uma granada com arrasto precisa de
/// integracao.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Fidelidade {
    /// Formula fechada. Custo constante por ponto, exata para gravidade
    /// uniforme sem arrasto.
    Analitica,
    /// Integracao passo a passo, semi-implicita. Aceita forcas arbitrarias e e
    /// o mesmo caminho que o integrador do mundo percorre.
    Integrada,
    /// Integrada com arrasto quadratico, `a = -k·|v|·v`.
    ///
    /// Diverge da analitica de proposito: com arrasto o alvo original deixa de
    /// ser alcancado, e e essa diferenca que a fidelidade compra.
    IntegradaComArrasto {
        /// Coeficiente de arrasto por unidade de massa.
        coeficiente: f32,
    },
}

impl Lancamento {
    /// Angulo de lancamento, em radianos.
    ///
    /// Conveniencia de apresentacao — para mostrar num editor ou num HUD. Passa
    /// por `atan2`, que vem da libm do sistema, entao **nao use o resultado
    /// dentro da simulacao** se determinismo entre plataformas importar.
    #[must_use]
    pub fn angulo(&self) -> f32 {
        self.velocidade_vertical.atan2(self.velocidade_horizontal)
    }

    /// Modulo da velocidade inicial.
    #[must_use]
    pub fn velocidade_inicial(&self) -> f32 {
        Vec2::new(self.velocidade_horizontal, self.velocidade_vertical).length()
    }

    /// Posicao no instante `t`, pela formula fechada.
    ///
    /// Relativa ao ponto de partida: `x` e o avanco horizontal, `y` a altura.
    #[must_use]
    pub fn posicao_em(&self, t: f32, gravidade: f32) -> Vec2 {
        Vec2::new(
            self.velocidade_horizontal * t,
            self.velocidade_vertical * t - 0.5 * gravidade * t * t,
        )
    }

    /// Percorre a trajetoria em `pontos` amostras igualmente espacadas no tempo,
    /// da partida ate a chegada.
    ///
    /// Sempre devolve ao menos dois pontos: partida e chegada.
    #[must_use]
    pub fn amostrar(&self, gravidade: f32, fidelidade: Fidelidade, pontos: u32) -> Vec<Vec2> {
        let n = pontos.max(2);
        // Indice como f32 sem `as`: acima de 65 535 pontos a amostragem satura
        // em vez de silenciosamente perder precisao.
        let indice = |i: u32| f32::from(u16::try_from(i).unwrap_or(u16::MAX));
        let dt = self.tempo_total / indice(n - 1);

        match fidelidade {
            Fidelidade::Analitica => {
                (0..n).map(|i| self.posicao_em(dt * indice(i), gravidade)).collect()
            }
            Fidelidade::Integrada => self.integrar(gravidade, dt, n, None),
            Fidelidade::IntegradaComArrasto { coeficiente } => {
                self.integrar(gravidade, dt, n, Some(coeficiente))
            }
        }
    }

    /// Integracao semi-implicita: a velocidade avanca primeiro, e a posicao usa
    /// a velocidade ja atualizada.
    ///
    /// E o mesmo esquema do integrador do mundo, de proposito — amostrar a
    /// trajetoria com um metodo e simular com outro produziria previsao que nao
    /// bate com o que acontece.
    fn integrar(&self, gravidade: f32, dt: f32, pontos: u32, arrasto: Option<f32>) -> Vec<Vec2> {
        let mut posicao = Vec2::ZERO;
        let mut velocidade = Vec2::new(self.velocidade_horizontal, self.velocidade_vertical);

        let mut saida = Vec::with_capacity(pontos as usize);
        saida.push(posicao);

        for _ in 1..pontos {
            let mut aceleracao = Vec2::new(0.0, -gravidade);
            if let Some(k) = arrasto {
                // Arrasto quadratico: opoe-se ao movimento e cresce com o
                // quadrado da velocidade.
                aceleracao -= velocidade * velocidade.length() * k;
            }

            velocidade += aceleracao * dt;
            posicao += velocidade * dt;
            saida.push(posicao);
        }

        saida
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn salto_padrao() -> Salto {
        Salto { altura_apice: 2.0, distancia: 6.0, desnivel: 0.0 }
    }

    // ------------------------------------------------------------ resolver --

    #[test]
    fn salto_simetrico_tem_subida_igual_a_descida() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");

        assert!((l.tempo_subida - l.tempo_descida).abs() < 1e-6);
        assert!((l.tempo_total - 2.0 * l.tempo_subida).abs() < 1e-6);
    }

    #[test]
    fn o_apice_pedido_e_o_apice_alcancado() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");
        let no_apice = l.posicao_em(l.tempo_subida, G_TERRA);

        assert!((no_apice.y - 2.0).abs() < 1e-4, "apice foi {}", no_apice.y);
    }

    #[test]
    fn a_distancia_pedida_e_a_distancia_percorrida() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");
        let na_chegada = l.posicao_em(l.tempo_total, G_TERRA);

        assert!((na_chegada.x - 6.0).abs() < 1e-4);
        assert!(na_chegada.y.abs() < 1e-3, "deveria voltar ao nivel de partida");
    }

    #[test]
    fn subir_para_uma_plataforma_encurta_a_descida() {
        let l = Salto { altura_apice: 3.0, distancia: 4.0, desnivel: 1.5 }
            .resolver(G_TERRA)
            .expect("possivel");

        assert!(l.tempo_descida < l.tempo_subida);
        let chegada = l.posicao_em(l.tempo_total, G_TERRA);
        assert!((chegada.y - 1.5).abs() < 1e-3, "chegou em {}", chegada.y);
    }

    #[test]
    fn descer_de_um_penhasco_alonga_a_descida() {
        let l = Salto { altura_apice: 1.0, distancia: 4.0, desnivel: -5.0 }
            .resolver(G_TERRA)
            .expect("possivel");

        assert!(l.tempo_descida > l.tempo_subida);
        let chegada = l.posicao_em(l.tempo_total, G_TERRA);
        assert!((chegada.y + 5.0).abs() < 1e-3);
    }

    #[test]
    fn salto_vertical_nao_avanca() {
        let l = Salto { altura_apice: 2.0, distancia: 0.0, desnivel: 0.0 }
            .resolver(G_TERRA)
            .expect("possivel");

        assert_eq!(l.velocidade_horizontal, 0.0);
        assert!(l.tempo_total > 0.0);
    }

    #[test]
    fn apice_maior_exige_mais_velocidade_vertical() {
        let baixo = Salto { altura_apice: 1.0, ..salto_padrao() }.resolver(G_TERRA).unwrap();
        let alto = Salto { altura_apice: 4.0, ..salto_padrao() }.resolver(G_TERRA).unwrap();

        assert!(alto.velocidade_vertical > baixo.velocidade_vertical);
        // E o voo mais longo deixa a horizontal mais lenta para a mesma
        // distancia.
        assert!(alto.velocidade_horizontal < baixo.velocidade_horizontal);
    }

    // ------------------------------------------------------------- recusas --

    #[test]
    fn gravidade_nao_positiva_e_recusada() {
        assert_eq!(salto_padrao().resolver(0.0), Err(SaltoImpossivel::SemGravidade));
        assert_eq!(salto_padrao().resolver(-1.0), Err(SaltoImpossivel::SemGravidade));
    }

    #[test]
    fn apice_que_nao_sobe_e_recusado() {
        let s = Salto { altura_apice: 0.0, ..salto_padrao() };
        assert_eq!(s.resolver(G_TERRA), Err(SaltoImpossivel::ApiceNaoSobe));
    }

    #[test]
    fn apice_abaixo_da_chegada_e_recusado() {
        // Pedir 1 m de apice para chegar a 2 m de altura nao tem solucao.
        let s = Salto { altura_apice: 1.0, distancia: 3.0, desnivel: 2.0 };
        assert_eq!(s.resolver(G_TERRA), Err(SaltoImpossivel::ApiceAbaixoDaChegada));
    }

    #[test]
    fn distancia_negativa_e_recusada() {
        let s = Salto { distancia: -1.0, ..salto_padrao() };
        assert_eq!(s.resolver(G_TERRA), Err(SaltoImpossivel::DistanciaNegativa));
    }

    // --------------------------------------------------------- fidelidades --

    /// Maior distancia entre a trajetoria fechada e a integrada, com `n`
    /// amostras.
    fn divergencia(l: &Lancamento, n: u32) -> f32 {
        let a = l.amostrar(G_TERRA, Fidelidade::Analitica, n);
        let b = l.amostrar(G_TERRA, Fidelidade::Integrada, n);
        a.iter().zip(&b).map(|(p, q)| p.distance(*q)).fold(0.0_f32, f32::max)
    }

    #[test]
    fn analitica_e_integrada_convergem() {
        // Criterio de aceite da Fase 5, com a tolerancia dita por extenso: com
        // 2048 amostras, a integracao fica a menos de 5 mm da formula fechada
        // num salto de 6 m — menos de 0,1% do alcance.
        //
        // A tolerancia nao e livre. A integracao e de primeira ordem, entao o
        // erro cai proporcionalmente ao passo e nao mais rapido: com 256
        // amostras a divergencia ainda e de 3 cm. Exigir 1 cm ali seria escrever
        // um teste que o metodo nao cumpre.
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");
        let pior = divergencia(&l, 2_048);

        assert!(pior < 0.005, "maior divergencia foi {pior} m");
    }

    #[test]
    fn o_erro_da_integracao_cai_junto_com_o_passo() {
        // Primeira ordem: dobrar as amostras corta o erro pela metade. E a
        // mesma propriedade verificada no integrador do ECS, e a razao e o que
        // denuncia uma troca silenciosa de metodo.
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");
        let razao = divergencia(&l, 512) / divergencia(&l, 1_024);

        assert!((1.8..2.2).contains(&razao), "esperado ~2, medido {razao}");
    }

    #[test]
    fn arrasto_encurta_o_alcance() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");

        let sem = l.amostrar(G_TERRA, Fidelidade::Analitica, 128);
        let com = l.amostrar(G_TERRA, Fidelidade::IntegradaComArrasto { coeficiente: 0.2 }, 128);

        let ultimo_sem = sem.last().expect("ha pontos");
        let ultimo_com = com.last().expect("ha pontos");

        assert!(ultimo_com.x < ultimo_sem.x, "o arrasto precisa frear o avanco");
        assert!(ultimo_com.y < ultimo_sem.y, "e derrubar a trajetoria mais cedo");
    }

    #[test]
    fn arrasto_zero_e_a_integracao_simples() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");

        let simples = l.amostrar(G_TERRA, Fidelidade::Integrada, 64);
        let sem_arrasto =
            l.amostrar(G_TERRA, Fidelidade::IntegradaComArrasto { coeficiente: 0.0 }, 64);

        assert_eq!(simples, sem_arrasto);
    }

    #[test]
    fn amostragem_sempre_tem_partida_e_chegada() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");

        for pedidos in [0, 1, 2, 7] {
            let pontos = l.amostrar(G_TERRA, Fidelidade::Analitica, pedidos);
            assert!(pontos.len() >= 2, "pedi {pedidos}, vieram {}", pontos.len());
            assert_eq!(pontos[0], Vec2::ZERO, "a primeira amostra e a partida");
        }
    }

    // -------------------------------------------------------- conveniencias --

    #[test]
    fn angulo_e_velocidade_descrevem_o_mesmo_lancamento() {
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");
        let v = l.velocidade_inicial();
        let a = l.angulo();

        // Reconstruir por modulo e angulo devolve as componentes originais.
        assert!((v * a.cos() - l.velocidade_horizontal).abs() < 1e-3);
        assert!((v * a.sin() - l.velocidade_vertical).abs() < 1e-3);
    }

    #[test]
    fn salto_mais_alto_que_longo_passa_de_45_graus() {
        let l = Salto { altura_apice: 5.0, distancia: 1.0, desnivel: 0.0 }
            .resolver(G_TERRA)
            .expect("possivel");

        assert!(l.angulo() > std::f32::consts::FRAC_PI_4);
    }

    // --------------------------------------------------------- determinismo --

    #[test]
    fn resolver_e_reproduzivel() {
        let a = salto_padrao().resolver(G_TERRA).expect("possivel");
        let b = salto_padrao().resolver(G_TERRA).expect("possivel");

        assert_eq!(a, b);
    }

    #[test]
    fn valores_de_referencia_ficam_fixados_bit_a_bit() {
        // Nao prova que a solucao evita trigonometria — nenhum teste prova. O
        // que ele faz e travar o resultado exato: uma reformulacao que passe a
        // usar `tan`/`atan` muda estes bits, e a mudanca aparece aqui em vez de
        // aparecer como divergencia de replay entre plataformas.
        let l = salto_padrao().resolver(G_TERRA).expect("possivel");

        assert_eq!(l.velocidade_vertical.to_bits(), 0x40c8_6b6f); // 6,263114 m/s
        assert_eq!(l.tempo_subida.to_bits(), 0x3f23_7f38); //          0,638660 s
        assert_eq!(l.tempo_descida.to_bits(), 0x3f23_7f37); //         0,638660 s
        assert_eq!(l.tempo_total.to_bits(), 0x3fa3_7f38); //           1,277320 s
        assert_eq!(l.velocidade_horizontal.to_bits(), 0x4096_5092); // 4,697335 m/s

        // Subida e descida saem de expressoes diferentes — sqrt(2gh)/g e
        // sqrt(2h/g) — e por isso diferem em 1 ULP mesmo num salto simetrico.
        // O teste de simetria acima usa tolerancia justamente por isso.
        assert_eq!(l.tempo_subida.to_bits() - l.tempo_descida.to_bits(), 1);
    }
}
