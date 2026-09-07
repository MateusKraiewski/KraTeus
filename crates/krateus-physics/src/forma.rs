//! Formas de colisao e o volume envolvente que a broadphase vai indexar.
//!
//! Este modulo e so geometria: nao ha corpo, nem contato, nem consulta. Ele
//! existe porque a broadphase precisa de uma caixa por forma antes de poder
//! ordenar qualquer coisa, e porque uma forma tem propriedades verificaveis
//! sozinha — o que o resto da cadeia de colisao nao tem.
//!
//! # Enum e nao trait
//!
//! A [D04](../../../docs/DECISOES.md) pede que a API publica da fisica seja
//! desenhada como trait, para que o Rapier possa entrar como backend. Isso vale
//! para a **costura do backend** — passo de simulacao e consultas —, nao para
//! cada forma individual.
//!
//! As formas sao um conjunto fechado e pequeno, percorrido no laco mais interno
//! da colisao. Um `dyn Forma` custaria despacho indireto ali dentro e, pior,
//! deixaria a ordem de despacho fora do controle do proprio codigo, que e
//! exatamente o que a [D09](../../../docs/DECISOES.md) exige poder garantir. O
//! proprio Rapier trata suas formas como conjunto fechado, e nao como objetos de
//! trait abertos.
//!
//! # AABB e OBB sao a mesma forma
//!
//! O roadmap lista "esfera, AABB, OBB, capsula, plano". AABB e OBB nao sao duas
//! formas: sao a mesma caixa, uma com rotacao identidade e outra sem. Modelar as
//! duas separadamente duplicaria todo teste de colisao que envolvesse caixa.
//! Aqui existe uma [`Forma::Caixa`], e a rotacao entra como parametro.
//!
//! # Determinismo
//!
//! Nada neste modulo chama funcao transcendental, e nem raiz quadrada. O volume
//! envolvente de uma caixa girada sai do truque da matriz em valor absoluto —
//! somas e produtos —, e as consultas de ponto comparam quadrados de distancia
//! em vez de distancias. E o que a D09 pede de codigo que alimenta a simulacao.

use glam::{Mat3, Quat, Vec3};

/// Caixa alinhada aos eixos do mundo.
///
/// E o volume envolvente que a broadphase compara, e nao uma forma de colisao:
/// duas AABB que se sobrepoem sao apenas um par que vale a pena examinar de
/// perto.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Aabb {
    /// Canto de menor coordenada em cada eixo.
    pub min: Vec3,
    /// Canto de maior coordenada em cada eixo.
    pub max: Vec3,
}

impl Aabb {
    /// Menor caixa que contem os dois pontos.
    ///
    /// A ordem dos argumentos nao importa: os cantos sao ordenados eixo a eixo.
    /// Nao ha, portanto, como construir uma caixa invertida por engano.
    #[must_use]
    pub fn de_extremos(a: Vec3, b: Vec3) -> Self {
        Self { min: a.min(b), max: a.max(b) }
    }

    /// Caixa a partir do centro e da metade da extensao em cada eixo.
    #[must_use]
    pub fn de_centro_e_meias_extensoes(centro: Vec3, meias_extensoes: Vec3) -> Self {
        Self { min: centro - meias_extensoes, max: centro + meias_extensoes }
    }

    /// Centro geometrico.
    #[must_use]
    pub fn centro(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Metade da extensao em cada eixo.
    #[must_use]
    pub fn meias_extensoes(&self) -> Vec3 {
        (self.max - self.min) * 0.5
    }

    /// Indica se o ponto esta dentro da caixa ou na sua fronteira.
    #[must_use]
    pub fn contem_ponto(&self, ponto: Vec3) -> bool {
        ponto.cmpge(self.min).all() && ponto.cmple(self.max).all()
    }

    /// Indica se as duas caixas se sobrepoem.
    ///
    /// **Encostar conta como sobrepor.** Dois corpos que se tocam exatamente sao
    /// o caso interessante para o contato, nao o caso a descartar; excluir a
    /// fronteira faria a broadphase perder justamente o par que o solver precisa
    /// examinar. O custo e um par a mais para a narrowphase, que e barato perto
    /// de um contato perdido.
    #[must_use]
    pub fn sobrepoe(&self, outra: &Self) -> bool {
        self.min.cmple(outra.max).all() && outra.min.cmple(self.max).all()
    }

    /// Menor caixa que contem as duas.
    #[must_use]
    pub fn uniao(&self, outra: &Self) -> Self {
        Self { min: self.min.min(outra.min), max: self.max.max(outra.max) }
    }

    /// Caixa afastada `margem` em cada direcao.
    #[must_use]
    pub fn expandida(&self, margem: f32) -> Self {
        let m = Vec3::splat(margem);
        Self { min: self.min - m, max: self.max + m }
    }
}

/// Uma forma de colisao, descrita no espaco local do corpo.
///
/// A posicao e a orientacao ficam de fora de proposito: quem as guarda e o
/// corpo, e uma mesma forma pode ser compartilhada por muitos deles.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum Forma {
    /// Esfera centrada na origem local.
    Esfera {
        /// Raio.
        raio: f32,
    },
    /// Caixa centrada na origem local.
    ///
    /// Com rotacao identidade e uma AABB; com qualquer outra rotacao, uma OBB.
    Caixa {
        /// Metade da extensao em cada eixo local.
        meias_extensoes: Vec3,
    },
    /// Capsula com o eixo em Y local.
    ///
    /// Y por convencao, e a mesma do personagem em pe. Uma capsula deitada e
    /// esta, girada.
    Capsula {
        /// Raio do cilindro e das duas calotas.
        raio: f32,
        /// Metade da distancia entre os centros das calotas.
        ///
        /// A altura total da capsula e `2 * (meia_altura + raio)`.
        meia_altura: f32,
    },
    /// Semiespaco: o solido e tudo que fica do lado oposto ao da normal.
    ///
    /// Nao e uma folha infinitamente fina, e sim o chao: um corpo que atravessa
    /// a superficie esta dentro do solido, e o contato tem para onde empurrar. O
    /// plano passa pela origem local — o deslocamento e a posicao do corpo, e
    /// guardar os dois seria guardar a mesma informacao duas vezes.
    Plano {
        /// Normal unitaria, apontando para fora do solido.
        normal: Vec3,
    },
}

impl Forma {
    /// Caixa alinhada ao mundo que envolve a forma na pose dada.
    ///
    /// Devolve `None` para o [`Plano`](Self::Plano), que e infinito. Nao ha
    /// caixa honesta a devolver ali: uma caixa infinita se sobreporia a todos os
    /// pares e transformaria a broadphase em forca bruta. O `None` obriga quem
    /// chama a tratar o semiespaco a parte, que e o que todo motor de fisica faz
    /// com o chao de qualquer maneira.
    #[must_use]
    pub fn aabb(&self, posicao: Vec3, rotacao: Quat) -> Option<Aabb> {
        match *self {
            Self::Esfera { raio } => {
                // Girar uma esfera nao muda nada, e por isso a rotacao nem entra
                // na conta.
                Some(Aabb::de_centro_e_meias_extensoes(posicao, Vec3::splat(raio)))
            }
            Self::Caixa { meias_extensoes } => {
                // Extensao de uma caixa girada: |M| aplicado as meias extensoes.
                // Cada eixo do mundo recebe a contribuicao dos tres eixos locais,
                // em valor absoluto porque o sinal so diz para que lado a caixa
                // se estende, e ela se estende para os dois.
                let m = Mat3::from_quat(rotacao);
                let extensao = m.x_axis.abs() * meias_extensoes.x
                    + m.y_axis.abs() * meias_extensoes.y
                    + m.z_axis.abs() * meias_extensoes.z;
                Some(Aabb::de_centro_e_meias_extensoes(posicao, extensao))
            }
            Self::Capsula { raio, meia_altura } => {
                // A capsula e o conjunto dos pontos a distancia `raio` de um
                // segmento. Envolver o segmento e engordar em `raio` da a caixa
                // exata, sem aproximar por esfera.
                let eixo = rotacao * (Vec3::Y * meia_altura);
                Some(Aabb::de_extremos(posicao - eixo, posicao + eixo).expandida(raio))
            }
            Self::Plano { .. } => None,
        }
    }

    /// Indica se um ponto **ja em espaco local** pertence ao solido.
    ///
    /// A fronteira conta como dentro, pelo mesmo motivo de
    /// [`Aabb::sobrepoe`].
    #[must_use]
    pub fn contem_ponto_local(&self, ponto: Vec3) -> bool {
        match *self {
            // Quadrados dos dois lados: a raiz nao mudaria a resposta e custaria
            // uma operacao que a D09 prefere evitar.
            Self::Esfera { raio } => ponto.length_squared() <= raio * raio,
            Self::Caixa { meias_extensoes } => ponto.abs().cmple(meias_extensoes).all(),
            Self::Capsula { raio, meia_altura } => {
                let no_eixo = Vec3::new(0.0, ponto.y.clamp(-meia_altura, meia_altura), 0.0);
                ponto.distance_squared(no_eixo) <= raio * raio
            }
            Self::Plano { normal } => ponto.dot(normal) <= 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_1_SQRT_2;

    /// Meia volta em torno de Y, exata em `f32`.
    ///
    /// Construida por componentes, e nao por `Quat::from_rotation_y`: aquela
    /// chama seno e cosseno, que sao justamente as funcoes que a D09 mantem
    /// fora da simulacao. Aqui os componentes sao literais, iguais nas duas
    /// plataformas por construcao.
    const MEIA_VOLTA_Y: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);

    /// Um quarto de volta em torno de Y. Os componentes sao constantes, mas o
    /// produto `2s²` nao fecha exatamente em 1, entao os testes que a usam
    /// comparam com tolerancia.
    const QUARTO_DE_VOLTA_Y: Quat = Quat::from_xyzw(0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2);

    fn perto(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() < 1e-5
    }

    // ----------------------------------------------------------------- aabb --

    #[test]
    fn de_extremos_ordena_os_cantos() {
        let a = Aabb::de_extremos(Vec3::new(3.0, -1.0, 5.0), Vec3::new(-2.0, 4.0, 0.0));

        assert_eq!(a.min, Vec3::new(-2.0, -1.0, 0.0));
        assert_eq!(a.max, Vec3::new(3.0, 4.0, 5.0));
    }

    #[test]
    fn centro_e_meias_extensoes_desfazem_a_construcao() {
        let centro = Vec3::new(1.0, -2.0, 3.0);
        let meias = Vec3::new(0.5, 1.5, 2.0);
        let a = Aabb::de_centro_e_meias_extensoes(centro, meias);

        assert_eq!(a.centro(), centro);
        assert_eq!(a.meias_extensoes(), meias);
    }

    #[test]
    fn contem_ponto_inclui_a_fronteira() {
        let a = Aabb::de_extremos(Vec3::ZERO, Vec3::ONE);

        assert!(a.contem_ponto(Vec3::splat(0.5)), "o centro esta dentro");
        assert!(a.contem_ponto(Vec3::ZERO), "o canto minimo esta dentro");
        assert!(a.contem_ponto(Vec3::ONE), "o canto maximo esta dentro");
        assert!(!a.contem_ponto(Vec3::new(1.0, 1.0, 1.001)));
    }

    #[test]
    fn sobreposicao_e_simetrica() {
        let a = Aabb::de_extremos(Vec3::ZERO, Vec3::splat(2.0));
        let b = Aabb::de_extremos(Vec3::ONE, Vec3::splat(3.0));

        assert!(a.sobrepoe(&b));
        assert!(b.sobrepoe(&a));
    }

    #[test]
    fn caixas_que_apenas_se_encostam_sobrepoem() {
        let a = Aabb::de_extremos(Vec3::ZERO, Vec3::ONE);
        let b = Aabb::de_extremos(Vec3::X, Vec3::new(2.0, 1.0, 1.0));

        assert!(a.sobrepoe(&b), "encostar conta, e o par vai para a narrowphase");
    }

    #[test]
    fn separacao_em_um_unico_eixo_ja_descarta_o_par() {
        let a = Aabb::de_extremos(Vec3::ZERO, Vec3::ONE);

        for deslocamento in [Vec3::X, Vec3::Y, Vec3::Z] {
            let b = Aabb::de_extremos(deslocamento * 1.5, deslocamento * 1.5 + Vec3::ONE);
            assert!(!a.sobrepoe(&b), "separadas em {deslocamento}");
            assert!(!b.sobrepoe(&a));
        }
    }

    #[test]
    fn uma_caixa_dentro_da_outra_sobrepoe() {
        let fora = Aabb::de_extremos(Vec3::splat(-5.0), Vec3::splat(5.0));
        let dentro = Aabb::de_extremos(Vec3::ZERO, Vec3::ONE);

        assert!(fora.sobrepoe(&dentro));
        assert!(dentro.sobrepoe(&fora));
    }

    #[test]
    fn a_uniao_contem_as_duas() {
        let a = Aabb::de_extremos(Vec3::new(-1.0, 0.0, 0.0), Vec3::ONE);
        let b = Aabb::de_extremos(Vec3::ZERO, Vec3::new(0.0, 4.0, 2.0));
        let u = a.uniao(&b);

        for canto in [a.min, a.max, b.min, b.max] {
            assert!(u.contem_ponto(canto), "a uniao perdeu {canto}");
        }
        assert_eq!(u.min, Vec3::new(-1.0, 0.0, 0.0));
        assert_eq!(u.max, Vec3::new(1.0, 4.0, 2.0));
    }

    #[test]
    fn expandida_cresce_para_os_dois_lados() {
        let a = Aabb::de_extremos(Vec3::ZERO, Vec3::ONE).expandida(0.25);

        assert_eq!(a.min, Vec3::splat(-0.25));
        assert_eq!(a.max, Vec3::splat(1.25));
    }

    // ---------------------------------------------------------------- aabb --
    //                                                            das formas

    #[test]
    fn a_aabb_da_esfera_e_justa() {
        let f = Forma::Esfera { raio: 2.0 };
        let a = f.aabb(Vec3::new(1.0, 0.0, -3.0), Quat::IDENTITY).expect("tem aabb");

        assert_eq!(a.min, Vec3::new(-1.0, -2.0, -5.0));
        assert_eq!(a.max, Vec3::new(3.0, 2.0, -1.0));
    }

    #[test]
    fn girar_a_esfera_nao_muda_a_aabb() {
        let f = Forma::Esfera { raio: 2.0 };
        let p = Vec3::new(1.0, 2.0, 3.0);

        assert_eq!(f.aabb(p, Quat::IDENTITY), f.aabb(p, QUARTO_DE_VOLTA_Y));
    }

    #[test]
    fn a_caixa_sem_rotacao_e_a_propria_aabb() {
        let meias = Vec3::new(1.0, 2.0, 3.0);
        let f = Forma::Caixa { meias_extensoes: meias };
        let a = f.aabb(Vec3::ZERO, Quat::IDENTITY).expect("tem aabb");

        assert_eq!(a.meias_extensoes(), meias);
    }

    #[test]
    fn meia_volta_devolve_exatamente_a_mesma_aabb() {
        // Meia volta leva cada eixo em si mesmo trocado de sinal, e o valor
        // absoluto desfaz a troca. A igualdade aqui e exata de proposito: se um
        // dia a conta passar a usar trigonometria, este e o teste que quebra.
        let f = Forma::Caixa { meias_extensoes: Vec3::new(1.0, 2.0, 3.0) };
        let p = Vec3::new(4.0, 5.0, 6.0);

        assert_eq!(f.aabb(p, MEIA_VOLTA_Y), f.aabb(p, Quat::IDENTITY));
    }

    #[test]
    fn um_quarto_de_volta_troca_x_com_z() {
        let f = Forma::Caixa { meias_extensoes: Vec3::new(1.0, 2.0, 3.0) };
        let a = f.aabb(Vec3::ZERO, QUARTO_DE_VOLTA_Y).expect("tem aabb");

        assert!(perto(a.meias_extensoes(), Vec3::new(3.0, 2.0, 1.0)), "{a:?}");
    }

    #[test]
    fn a_caixa_girada_45_graus_cresce() {
        // Um quarto de volta apenas permuta eixos; e a 45 graus que a AABB de uma
        // caixa fica maior que a caixa, e e por isso que ela nao serve como forma
        // de colisao, so como filtro.
        let meias = Vec3::new(1.0, 1.0, 1.0);
        let f = Forma::Caixa { meias_extensoes: meias };
        let meio_quarto = Quat::from_xyzw(0.0, 0.382_683_43, 0.0, 0.923_879_5);
        let a = f.aabb(Vec3::ZERO, meio_quarto).expect("tem aabb");

        assert!(a.meias_extensoes().x > 1.4, "x = {}", a.meias_extensoes().x);
        assert!(a.meias_extensoes().z > 1.4, "z = {}", a.meias_extensoes().z);
        assert!((a.meias_extensoes().y - 1.0).abs() < 1e-5, "Y nao deveria mudar");
    }

    #[test]
    fn a_capsula_em_pe_soma_raio_e_meia_altura() {
        let f = Forma::Capsula { raio: 0.5, meia_altura: 1.0 };
        let a = f.aabb(Vec3::ZERO, Quat::IDENTITY).expect("tem aabb");

        assert_eq!(a.meias_extensoes(), Vec3::new(0.5, 1.5, 0.5));
    }

    #[test]
    fn a_capsula_deitada_troca_os_eixos() {
        // Um quarto de volta em torno de Z leva o eixo Y para X.
        let quarto_z = Quat::from_xyzw(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2);
        let f = Forma::Capsula { raio: 0.5, meia_altura: 1.0 };
        let a = f.aabb(Vec3::ZERO, quarto_z).expect("tem aabb");

        assert!(perto(a.meias_extensoes(), Vec3::new(1.5, 0.5, 0.5)), "{a:?}");
    }

    #[test]
    fn o_plano_nao_tem_aabb() {
        let f = Forma::Plano { normal: Vec3::Y };

        assert_eq!(f.aabb(Vec3::ZERO, Quat::IDENTITY), None);
        assert_eq!(f.aabb(Vec3::splat(9.0), QUARTO_DE_VOLTA_Y), None);
    }

    #[test]
    fn a_aabb_acompanha_a_posicao() {
        let f = Forma::Capsula { raio: 0.5, meia_altura: 1.0 };
        let origem = f.aabb(Vec3::ZERO, Quat::IDENTITY).expect("tem aabb");
        let deslocada = f.aabb(Vec3::new(10.0, -4.0, 2.0), Quat::IDENTITY).expect("tem aabb");

        assert_eq!(deslocada.centro() - origem.centro(), Vec3::new(10.0, -4.0, 2.0));
        assert_eq!(deslocada.meias_extensoes(), origem.meias_extensoes());
    }

    // ------------------------------------------------------- ponto e solido --

    #[test]
    fn a_esfera_contem_ate_a_superficie() {
        let f = Forma::Esfera { raio: 2.0 };

        assert!(f.contem_ponto_local(Vec3::ZERO));
        assert!(f.contem_ponto_local(Vec3::X * 2.0), "a superficie conta");
        assert!(!f.contem_ponto_local(Vec3::X * 2.001));
    }

    #[test]
    fn a_caixa_contem_ate_o_canto() {
        let meias = Vec3::new(1.0, 2.0, 3.0);
        let f = Forma::Caixa { meias_extensoes: meias };

        assert!(f.contem_ponto_local(meias), "o canto conta");
        assert!(f.contem_ponto_local(-meias));
        assert!(!f.contem_ponto_local(meias + Vec3::X * 0.001));
    }

    #[test]
    fn a_capsula_e_um_segmento_engordado() {
        let f = Forma::Capsula { raio: 0.5, meia_altura: 1.0 };

        assert!(f.contem_ponto_local(Vec3::ZERO), "o eixo esta dentro");
        assert!(f.contem_ponto_local(Vec3::new(0.5, 0.0, 0.0)), "a lateral do cilindro");
        assert!(f.contem_ponto_local(Vec3::new(0.0, 1.5, 0.0)), "o topo da calota");
        assert!(!f.contem_ponto_local(Vec3::new(0.0, 1.501, 0.0)));
        // O canto de uma caixa de mesmas dimensoes fica de fora: e a diferenca
        // entre capsula e caixa.
        assert!(!f.contem_ponto_local(Vec3::new(0.5, 1.5, 0.0)));
    }

    #[test]
    fn o_plano_e_um_semiespaco() {
        let f = Forma::Plano { normal: Vec3::Y };

        assert!(f.contem_ponto_local(Vec3::NEG_Y), "abaixo do chao e solido");
        assert!(f.contem_ponto_local(Vec3::ZERO), "a superficie conta");
        assert!(!f.contem_ponto_local(Vec3::Y), "acima do chao e vazio");
    }

    // ------------------------------------------------ a propriedade central --

    #[test]
    fn a_aabb_contem_todos_os_pontos_da_forma() {
        // A unica propriedade que a AABB precisa cumprir para servir de filtro:
        // nada da forma pode ficar de fora. Um envelope que perde um ponto faz a
        // broadphase descartar um par que estava em contato.
        let posicao = Vec3::new(2.0, -1.0, 0.5);
        let formas = [
            Forma::Esfera { raio: 1.5 },
            Forma::Caixa { meias_extensoes: Vec3::new(1.0, 0.5, 2.0) },
            Forma::Capsula { raio: 0.4, meia_altura: 1.2 },
        ];

        for forma in formas {
            for rotacao in [Quat::IDENTITY, MEIA_VOLTA_Y, QUARTO_DE_VOLTA_Y] {
                let aabb = forma.aabb(posicao, rotacao).expect("tem aabb");
                let mut testados = 0;

                // i16 e nao i32 porque `f32::from` so aceita tipos em que a
                // conversao nao pode perder precisao — e converter com `as`
                // esconderia justamente esse cuidado.
                for i in -30i16..=30 {
                    for j in -30i16..=30 {
                        for k in -30i16..=30 {
                            let local = Vec3::new(f32::from(i), f32::from(j), f32::from(k)) * 0.1;
                            if !forma.contem_ponto_local(local) {
                                continue;
                            }
                            testados += 1;
                            let mundo = posicao + rotacao * local;
                            // A margem cobre o arredondamento da transformacao:
                            // um ponto exatamente na fronteira pode cair um ulp
                            // para fora depois de girado.
                            assert!(
                                aabb.expandida(1e-5).contem_ponto(mundo),
                                "{forma:?} perdeu {local} -> {mundo}, aabb {aabb:?}"
                            );
                        }
                    }
                }

                assert!(testados > 100, "{forma:?} mal foi amostrada: {testados} pontos");
            }
        }
    }

    // --------------------------------------------------------- determinismo --

    #[test]
    fn a_mesma_pose_produz_a_mesma_aabb_bit_a_bit() {
        let f = Forma::Caixa { meias_extensoes: Vec3::new(1.0, 2.0, 3.0) };
        let p = Vec3::new(0.1, 0.2, 0.3);

        let a = f.aabb(p, QUARTO_DE_VOLTA_Y).expect("tem aabb");
        let b = f.aabb(p, QUARTO_DE_VOLTA_Y).expect("tem aabb");

        assert_eq!(a.min.to_array().map(f32::to_bits), b.min.to_array().map(f32::to_bits));
        assert_eq!(a.max.to_array().map(f32::to_bits), b.max.to_array().map(f32::to_bits));
    }
}
