//! Broadphase: quais pares de corpos valem a pena examinar de perto.
//!
//! A broadphase nao decide se dois corpos se tocam. Ela descarta, barato, a
//! maioria esmagadora dos pares que **nao** podem estar em contato, e entrega o
//! resto para a narrowphase — que ainda nao existe. O criterio de descarte e o
//! volume envolvente da [`Forma`](crate::Forma), e nao a forma em si.
//!
//! # Sweep and prune em lote
//!
//! Ordena os corpos pelo inicio do envelope num eixo e varre uma vez, mantendo
//! a lista dos que ainda nao terminaram. Dois corpos que nao se sobrepoem
//! naquele eixo nunca chegam a ser comparados.
//!
//! "Em lote" quer dizer sem estado entre quadros: cada chamada recebe a cena
//! inteira e devolve os pares, sem guardar nada. E o mesmo desenho do Render
//! World, que refaz o snapshot a cada quadro em vez de manter cache.
//!
//! ## Por que nao um grid
//!
//! Um grid exige escolher um tamanho de celula, e nao existe cena real que
//! justifique um valor hoje. Pior: o numero contaminaria os testes, que
//! passariam todos a depender dele. O SAP nao tem parametro — nao ha o que
//! ajustar, e nao ha o que ajustar errado.
//!
//! O grid paralelizaria melhor. Isso pesa menos do que parece: o trabalho
//! paralelizavel de verdade e a narrowphase, que testa os pares e e
//! embaracosamente paralela venha de onde vier o par.
//!
//! ## O que este desenho **nao** faz, de proposito
//!
//! Nao ha SAP incremental, nem tratamento especial de corpos estaticos. Um
//! corpo parado e reordenado a cada chamada como qualquer outro. Sao otimizacoes
//! reais, e ambas dependem de medicao para valer — a Fase 7 e quem mede.
//!
//! # Determinismo
//!
//! [`pares`] e funcao pura: mesma entrada, mesma saida, em qualquer plataforma.
//! Tres cuidados sustentam isso, e cada um esta comentado onde acontece:
//!
//! - a ordem da soma na variancia faz parte do resultado, porque soma de `f32`
//!   nao e associativa;
//! - a ordenacao usa [`f32::total_cmp`], que e ordem total de verdade, e desempata
//!   por [`Entity`], que a documentacao do proprio tipo diz existir para isso;
//! - a saida e canonica: par ordenado, lista ordenada, sem duplicata.
//!
//! Nenhuma estrutura de ordem de iteracao indefinida aparece aqui — nada de
//! `HashMap`, como a D09 exige.

use glam::{Quat, Vec3};
use krateus_ecs::{Entity, World};

use crate::forma::{Aabb, Forma};
use crate::integrador::Posicao;

/// Liga uma entidade a sua forma de colisao.
///
/// So isso. A pose vem da [`Posicao`], a massa da
/// [`MassaInversa`](crate::MassaInversa), e nada mais entra aqui: um colisor que
/// tambem guardasse material, camada ou filtro seria tres decisoes tomadas sem
/// um caso de uso que as exija.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Colisor(pub Forma);

/// Um dos tres eixos do mundo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eixo {
    /// Eixo X.
    X,
    /// Eixo Y.
    Y,
    /// Eixo Z.
    Z,
}

impl Eixo {
    /// Componente do vetor neste eixo.
    #[must_use]
    pub fn componente(self, v: Vec3) -> f32 {
        match self {
            Self::X => v.x,
            Self::Y => v.y,
            Self::Z => v.z,
        }
    }
}

/// Envelopes dos corpos do mundo, na ordem de iteracao do ECS.
///
/// Corpos cuja forma nao tem envelope finito — o
/// [`Plano`](crate::Forma::Plano), pela [D14](../../../docs/DECISOES.md) — ficam
/// de fora. Nao e um esquecimento: um semiespaco nao entra numa broadphase, ele
/// e testado a parte contra todo mundo. Enquanto nao houver narrowphase, nao ha
/// onde fazer isso, e inventar o lugar agora seria adivinhar a forma.
///
/// **A orientacao e sempre a identidade.** A fisica ainda nao tem componente de
/// rotacao porque ainda nao tem dinamica angular — sem torque e sem tensor de
/// inercia, uma rotacao seria um dado que nada atualiza (a mesma razao que a
/// [D13](../../../docs/DECISOES.md) da para nao ter escala). Uma
/// [`Forma::Caixa`] no mundo e, portanto, alinhada aos eixos hoje; o caminho da
/// caixa girada existe e e testado em [`crate::forma`], esperando o corpo rigido.
#[must_use]
pub fn envelopes(world: &mut World) -> Vec<(Entity, Aabb)> {
    world
        .query::<(Entity, &Posicao, &Colisor)>()
        .filter_map(|(entidade, posicao, colisor)| {
            colisor.0.aabb(posicao.0, Quat::IDENTITY).map(|caixa| (entidade, caixa))
        })
        .collect()
}

/// Eixo em que os corpos estao mais espalhados.
///
/// Varrer o eixo de maior dispersao e o que mantem a lista de ativos curta. Num
/// mundo chao — quase tudo na mesma altura —, varrer Y compararia praticamente
/// todos contra todos.
///
/// Publico por servir a diagnostico e a teste, como
/// [`Schedule::batches`](krateus_ecs::Schedule::batches): e assim que se confere
/// que a varredura escolheu o eixo que se esperava.
///
/// # Determinismo
///
/// A media e a dispersao sao somas de `f32`, e **soma de `f32` nao e
/// associativa**: trocar a ordem das parcelas muda o ultimo bit do resultado, o
/// que pode mudar o eixo escolhido num empate apertado e, com ele, a ordem em
/// que os pares sao gerados. Por isso o laco percorre `corpos` na ordem em que
/// veio, e cabe a quem chama que essa ordem seja estavel —
/// [`envelopes`] garante isso usando a ordem de iteracao do ECS.
///
/// Empate exato entre eixos resolve em X, depois Y, depois Z.
#[must_use]
pub fn eixo_de_maior_variancia(corpos: &[(Entity, Aabb)]) -> Eixo {
    if corpos.is_empty() {
        return Eixo::X;
    }

    // `as f32` numa contagem: nao ha conversao infalivel de `usize`, e uma cena
    // com mais de 2^24 corpos tem problemas maiores que o arredondamento aqui.
    let inverso_de_n = 1.0 / corpos.len() as f32;

    let mut soma = Vec3::ZERO;
    for (_, caixa) in corpos {
        soma += caixa.centro();
    }
    let media = soma * inverso_de_n;

    let mut dispersao = Vec3::ZERO;
    for (_, caixa) in corpos {
        let desvio = caixa.centro() - media;
        dispersao += desvio * desvio;
    }

    // A divisao final por `n` fica de fora: e o mesmo fator nos tres eixos, e
    // nao muda qual e o maior. Uma operacao a menos e um arredondamento a menos.
    if dispersao.x >= dispersao.y && dispersao.x >= dispersao.z {
        Eixo::X
    } else if dispersao.y >= dispersao.z {
        Eixo::Y
    } else {
        Eixo::Z
    }
}

/// Pares de corpos cujos envelopes se sobrepoem.
///
/// A interface e deliberadamente independente do algoritmo: entra uma fatia de
/// `(Entity, Aabb)`, sai a lista de pares. Trocar sweep and prune por um grid
/// depois e trocar o corpo desta funcao, com os mesmos testes valendo.
///
/// # Forma da saida
///
/// - **Cada par aparece uma vez.** `(A, B)` e `(B, A)` sao o mesmo par.
/// - **Dentro do par**, a entidade menor vem primeiro, na ordem total que o
///   [`Entity`] define: indice, depois geracao.
/// - **Entre pares**, a lista e ordenada por essa mesma ordem, primeiro elemento
///   e depois segundo.
///
/// A consequencia util e que permutar a entrada nao muda a lista: os pares sao
/// canonizados e a lista e ordenada no fim. A unica via por onde a ordem de
/// entrada ainda pode influir e o ultimo bit da variancia, num empate apertado
/// entre eixos — ver [`eixo_de_maior_variancia`]. Trocar de eixo nao muda
/// *quais* pares saem, so o caminho ate eles.
#[must_use]
pub fn pares(corpos: &[(Entity, Aabb)]) -> Vec<(Entity, Entity)> {
    if corpos.len() < 2 {
        return Vec::new();
    }

    let eixo = eixo_de_maior_variancia(corpos);

    // Ordena indices, e nao os corpos: a fatia de entrada fica intocada, e quem
    // chama nao precisa de uma copia so para consultar a broadphase.
    let mut ordem: Vec<usize> = (0..corpos.len()).collect();
    ordem.sort_by(|&a, &b| {
        // `total_cmp` e ordem total de verdade, inclusive sobre NaN. Comparador
        // que nao seja ordem total faz `sort_by` entrar em panico desde a 1.81 —
        // e `partial_cmp` nao e, exatamente por causa do NaN.
        eixo.componente(corpos[a].1.min)
            .total_cmp(&eixo.componente(corpos[b].1.min))
            .then(corpos[a].0.cmp(&corpos[b].0))
    });

    let mut saida = Vec::new();
    let mut ativos: Vec<usize> = Vec::new();

    for &i in &ordem {
        let (entidade, caixa) = corpos[i];
        let inicio = eixo.componente(caixa.min);

        // Sai da lista quem termina antes deste comecar. `>=` e nao `>` porque
        // encostar conta como sobrepor, a mesma regra de [`Aabb::sobrepoe`].
        ativos.retain(|&j| eixo.componente(corpos[j].1.max) >= inicio);

        for &j in &ativos {
            let (outra, caixa_outra) = corpos[j];
            // Uma entidade repetida na entrada nao vira par consigo mesma.
            if entidade != outra && caixa.sobrepoe(&caixa_outra) {
                saida.push(par_canonico(entidade, outra));
            }
        }

        ativos.push(i);
    }

    saida.sort_unstable();
    // A varredura nao gera par repetido: cada par sai uma vez, quando o segundo
    // corpo chega. A limpeza cobre entrada com corpo repetido, que a funcao
    // aceita por ser publica.
    saida.dedup();
    saida
}

/// Par com a entidade menor primeiro.
fn par_canonico(a: Entity, b: Entity) -> (Entity, Entity) {
    if a <= b { (a, b) } else { (b, a) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::num::NonZeroU32;

    /// Indices em `u16` para que `f32::from` sirva: nao ha conversao infalivel
    /// de `u32` para `f32`, e usar `as` num teste esconderia o cuidado que o
    /// resto do modulo toma.
    fn ent(indice: u16) -> Entity {
        Entity::from_raw(u32::from(indice), NonZeroU32::MIN)
    }

    /// Cubo de lado 1 centrado no ponto dado.
    fn corpo(indice: u16, centro: Vec3) -> (Entity, Aabb) {
        (ent(indice), Aabb::de_centro_e_meias_extensoes(centro, Vec3::splat(0.5)))
    }

    fn caixa(centro: Vec3, meias: Vec3) -> Aabb {
        Aabb::de_centro_e_meias_extensoes(centro, meias)
    }

    /// Todos contra todos. E a definicao do resultado correto; o SAP so precisa
    /// chegar nela mais rapido.
    fn forca_bruta(corpos: &[(Entity, Aabb)]) -> Vec<(Entity, Entity)> {
        let mut saida = Vec::new();
        for (i, (ea, ca)) in corpos.iter().enumerate() {
            for (eb, cb) in &corpos[i + 1..] {
                if ea != eb && ca.sobrepoe(cb) {
                    saida.push(par_canonico(*ea, *eb));
                }
            }
        }
        saida.sort_unstable();
        saida.dedup();
        saida
    }

    /// xorshift64*, para cenas reproduziveis sem depender de gerador externo.
    struct Aleatorio(u64);

    impl Aleatorio {
        fn proximo(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
        }

        /// Coordenada em passos de 1/16, dentro de `[-passos, passos] / 16`.
        ///
        /// Passos discretos, e nao float continuo, para que coordenadas iguais
        /// aparecam com frequencia — e o caso que castiga a ordenacao.
        fn coordenada(&mut self, passos: u16) -> f32 {
            let faixa = u64::from(passos) * 2 + 1;
            let sorteado =
                u16::try_from(self.proximo() % faixa).expect("faixa pequena por construcao");
            (f32::from(sorteado) - f32::from(passos)) / 16.0
        }
    }

    // --------------------------------------------------------- casos basicos --

    #[test]
    fn cena_vazia_nao_tem_pares() {
        assert!(pares(&[]).is_empty());
    }

    #[test]
    fn um_corpo_sozinho_nao_tem_par() {
        assert!(pares(&[corpo(0, Vec3::ZERO)]).is_empty());
    }

    #[test]
    fn dois_corpos_sobrepostos_formam_um_par() {
        let cena = [corpo(0, Vec3::ZERO), corpo(1, Vec3::X * 0.5)];

        assert_eq!(pares(&cena), vec![(ent(0), ent(1))]);
    }

    #[test]
    fn dois_corpos_separados_nao_formam_par() {
        let cena = [corpo(0, Vec3::ZERO), corpo(1, Vec3::X * 5.0)];

        assert!(pares(&cena).is_empty());
    }

    #[test]
    fn corpos_que_apenas_se_encostam_formam_par() {
        // Mesma regra de Aabb::sobrepoe: o par vai adiante e a narrowphase
        // decide. Perder um contato custa mais que examinar um par a mais.
        let cena = [corpo(0, Vec3::ZERO), corpo(1, Vec3::X)];

        assert_eq!(pares(&cena), vec![(ent(0), ent(1))]);
    }

    #[test]
    fn uma_caixa_dentro_da_outra_forma_par() {
        let cena = [
            (ent(0), caixa(Vec3::ZERO, Vec3::splat(10.0))),
            (ent(1), caixa(Vec3::ZERO, Vec3::splat(0.1))),
        ];

        assert_eq!(pares(&cena), vec![(ent(0), ent(1))]);
    }

    #[test]
    fn caixa_grande_envolvendo_varias_pequenas() {
        // O caso que mais castiga a varredura: a caixa grande entra na lista de
        // ativos no comeco e so sai no fim, sendo comparada com todas.
        let mut cena = vec![(ent(0), caixa(Vec3::ZERO, Vec3::splat(20.0)))];
        for i in 1..=8u16 {
            cena.push(corpo(i, Vec3::X * (f32::from(i) * 2.0)));
        }

        let esperado: Vec<_> = (1..=8u16).map(|i| (ent(0), ent(i))).collect();
        assert_eq!(pares(&cena), esperado);
        assert_eq!(pares(&cena), forca_bruta(&cena));
    }

    #[test]
    fn varios_corpos_com_varios_pares() {
        // Tres em contato em cadeia, e um quarto longe.
        let cena = [
            corpo(0, Vec3::ZERO),
            corpo(1, Vec3::X * 0.4),
            corpo(2, Vec3::X * 0.8),
            corpo(3, Vec3::X * 100.0),
        ];

        assert_eq!(pares(&cena), vec![(ent(0), ent(1)), (ent(0), ent(2)), (ent(1), ent(2))]);
    }

    // ----------------------------------------------------------- degeneradas --

    #[test]
    fn coordenadas_iguais_no_eixo_varrido() {
        // Pares de corpos exatamente na mesma altura: a chave de ordenacao
        // empata, e quem desempata e a Entity. X e Z tem dispersao zero, entao a
        // varredura acontece justamente em Y, onde estao os empates.
        let cena: Vec<_> =
            (0..6u16).map(|i| corpo(i, Vec3::Y * (f32::from(i / 2) * 0.4))).collect();

        assert_eq!(eixo_de_maior_variancia(&cena), Eixo::Y);
        assert_eq!(pares(&cena), forca_bruta(&cena));
        assert!(!pares(&cena).is_empty());
    }

    #[test]
    fn todos_no_mesmo_lugar() {
        // Dispersao zero nos tres eixos: o desempate escolhe X, e todos os pares
        // saem.
        let cena: Vec<_> = (0..5u16).map(|i| corpo(i, Vec3::ZERO)).collect();

        assert_eq!(eixo_de_maior_variancia(&cena), Eixo::X);
        assert_eq!(pares(&cena).len(), 10, "5 corpos no mesmo ponto sao 10 pares");
        assert_eq!(pares(&cena), forca_bruta(&cena));
    }

    #[test]
    fn entidade_repetida_na_entrada_nao_vira_par_consigo_mesma() {
        let cena = [corpo(7, Vec3::ZERO), corpo(7, Vec3::ZERO)];

        assert!(pares(&cena).is_empty());
    }

    // ------------------------------------------------------ eixo da varredura --

    #[test]
    fn o_eixo_escolhido_e_o_de_maior_dispersao() {
        let em_x: Vec<_> = (0..5u16).map(|i| corpo(i, Vec3::X * f32::from(i))).collect();
        let em_y: Vec<_> = (0..5u16).map(|i| corpo(i, Vec3::Y * f32::from(i))).collect();
        let em_z: Vec<_> = (0..5u16).map(|i| corpo(i, Vec3::Z * f32::from(i))).collect();

        assert_eq!(eixo_de_maior_variancia(&em_x), Eixo::X);
        assert_eq!(eixo_de_maior_variancia(&em_y), Eixo::Y);
        assert_eq!(eixo_de_maior_variancia(&em_z), Eixo::Z);
    }

    #[test]
    fn cena_vazia_escolhe_x() {
        assert_eq!(eixo_de_maior_variancia(&[]), Eixo::X);
    }

    #[test]
    fn empate_exato_entre_os_tres_eixos_resolve_em_x() {
        // Quatro cantos de um cubo: a dispersao da exatamente igual nos tres
        // eixos, e em f32 sem arredondamento. A escolha do eixo entra na ordem em
        // que os pares sao gerados, entao o desempate faz parte do contrato.
        let cena = [
            corpo(0, Vec3::new(-1.0, -1.0, -1.0)),
            corpo(1, Vec3::new(1.0, 1.0, 1.0)),
            corpo(2, Vec3::new(-1.0, 1.0, -1.0)),
            corpo(3, Vec3::new(1.0, -1.0, 1.0)),
        ];

        assert_eq!(eixo_de_maior_variancia(&cena), Eixo::X);
    }

    #[test]
    fn o_eixo_muda_o_caminho_e_nao_o_resultado() {
        // Cena chata em X e em Y, espalhada em Z: a varredura escolhe Z, e o
        // resultado ainda tem de bater com a forca bruta.
        let cena: Vec<_> =
            (0..12u16).map(|i| corpo(i, Vec3::new(0.1, 0.0, f32::from(i) * 0.45))).collect();

        assert_eq!(eixo_de_maior_variancia(&cena), Eixo::Z);
        assert_eq!(pares(&cena), forca_bruta(&cena));
    }

    // --------------------------------------------------------- forma da saida --

    #[test]
    fn a_saida_nao_tem_duplicatas() {
        let cena: Vec<_> = (0..20u16).map(|i| corpo(i, Vec3::X * (f32::from(i) * 0.3))).collect();
        let mut p = pares(&cena);
        let antes = p.len();

        p.dedup();
        assert_eq!(p.len(), antes, "a broadphase repetiu um par");
        assert!(antes > 0, "a cena precisa gerar pares para o teste valer");
    }

    #[test]
    fn dentro_do_par_a_entidade_menor_vem_primeiro() {
        // A entrada esta em ordem decrescente de entidade, de proposito.
        let cena = [corpo(9, Vec3::ZERO), corpo(2, Vec3::X * 0.5)];

        assert_eq!(pares(&cena), vec![(ent(2), ent(9))]);
    }

    #[test]
    fn a_lista_sai_ordenada() {
        let cena: Vec<_> = (0..15u16).map(|i| corpo(i, Vec3::X * (f32::from(i) * 0.3))).collect();
        let p = pares(&cena);
        let mut ordenada = p.clone();
        ordenada.sort_unstable();

        assert_eq!(p, ordenada);
        assert!(!p.is_empty());
    }

    #[test]
    fn permutar_a_entrada_nao_muda_a_saida() {
        let base: Vec<_> = (0..16u16).map(|i| corpo(i, Vec3::X * (f32::from(i) * 0.35))).collect();
        let esperado = pares(&base);
        assert!(!esperado.is_empty(), "a cena precisa gerar pares");

        // Inverter e embaralhar sao duas permutacoes diferentes da mesma cena.
        let invertida: Vec<_> = base.iter().copied().rev().collect();
        assert_eq!(pares(&invertida), esperado);

        let mut embaralhada = base.clone();
        let mut r = Aleatorio(0x5eed_0f0f_1234_0001);
        for i in (1..embaralhada.len()).rev() {
            let limite = u64::try_from(i).expect("indice cabe em u64") + 1;
            let j = usize::try_from(r.proximo() % limite).expect("menor que o indice");
            embaralhada.swap(i, j);
        }
        assert_eq!(pares(&embaralhada), esperado);
    }

    // ----------------------------------------------------------- determinismo --

    #[test]
    fn a_mesma_cena_produz_a_mesma_saida() {
        let mut r = Aleatorio(0x5eed_1234_abcd_0007);
        let cena: Vec<_> = (0..40u16)
            .map(|i| {
                let c = Vec3::new(r.coordenada(128), r.coordenada(128), r.coordenada(128));
                corpo(i, c)
            })
            .collect();

        assert_eq!(pares(&cena), pares(&cena));
        assert_eq!(eixo_de_maior_variancia(&cena), eixo_de_maior_variancia(&cena));
        assert!(!pares(&cena).is_empty());
    }

    // --------------------------------------------------- contra a forca bruta --

    #[test]
    fn concorda_com_a_forca_bruta_em_cenas_aleatorias() {
        // O teste que sustenta todos os outros: o SAP pode ser mais rapido,
        // nunca diferente.
        for semente in [1_u64, 2, 3, 5, 8, 13, 21] {
            for n in [2_u16, 3, 7, 20, 55] {
                let mut r = Aleatorio(semente.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
                let cena: Vec<_> = (0..n)
                    .map(|i| {
                        let centro =
                            Vec3::new(r.coordenada(96), r.coordenada(96), r.coordenada(96));
                        let meias = Vec3::new(
                            0.25 + r.coordenada(12).abs(),
                            0.25 + r.coordenada(12).abs(),
                            0.25 + r.coordenada(12).abs(),
                        );
                        (ent(i), Aabb::de_centro_e_meias_extensoes(centro, meias))
                    })
                    .collect();

                assert_eq!(pares(&cena), forca_bruta(&cena), "semente {semente}, {n} corpos");
            }
        }
    }

    #[test]
    fn concorda_com_a_forca_bruta_num_mundo_chato() {
        // Tudo na mesma altura, o pior caso para varrer Y. A escolha por
        // variancia deve evitar Y; o resultado bate de qualquer jeito.
        let mut r = Aleatorio(0xabcd_0000_0000_0001);
        let cena: Vec<_> = (0..50u16)
            .map(|i| corpo(i, Vec3::new(r.coordenada(160), 0.0, r.coordenada(160))))
            .collect();

        assert_ne!(eixo_de_maior_variancia(&cena), Eixo::Y);
        assert_eq!(pares(&cena), forca_bruta(&cena));
    }

    // ------------------------------------------------------ ligacao com o ECS --

    #[test]
    fn envelopes_sai_do_mundo_com_a_forma_de_cada_corpo() {
        let mut world = World::new();
        let a = world.spawn((Posicao(Vec3::ZERO), Colisor(Forma::Esfera { raio: 1.0 })));
        let b = world.spawn((
            Posicao(Vec3::X * 10.0),
            Colisor(Forma::Caixa { meias_extensoes: Vec3::splat(2.0) }),
        ));

        let mut envs = envelopes(&mut world);
        envs.sort_by_key(|(e, _)| *e);

        assert_eq!(envs.len(), 2);
        assert_eq!(envs[0].0, a);
        assert_eq!(envs[0].1, Aabb::de_centro_e_meias_extensoes(Vec3::ZERO, Vec3::ONE));
        assert_eq!(envs[1].0, b);
        let esperada = Aabb::de_centro_e_meias_extensoes(Vec3::X * 10.0, Vec3::splat(2.0));
        assert_eq!(envs[1].1, esperada);
    }

    #[test]
    fn corpo_sem_colisor_nao_entra_na_broadphase() {
        let mut world = World::new();
        world.spawn((Posicao(Vec3::ZERO),));
        let com = world.spawn((Posicao(Vec3::ZERO), Colisor(Forma::Esfera { raio: 1.0 })));

        let envs = envelopes(&mut world);

        assert_eq!(envs.len(), 1);
        assert_eq!(envs[0].0, com);
    }

    #[test]
    fn o_plano_fica_de_fora_dos_envelopes() {
        let mut world = World::new();
        world.spawn((Posicao(Vec3::ZERO), Colisor(Forma::Plano { normal: Vec3::Y })));
        let esfera = world.spawn((Posicao(Vec3::ZERO), Colisor(Forma::Esfera { raio: 1.0 })));

        let envs = envelopes(&mut world);

        assert_eq!(envs.len(), 1, "o semiespaco nao tem envelope finito");
        assert_eq!(envs[0].0, esfera);
    }

    #[test]
    fn do_mundo_ate_os_pares() {
        let mut world = World::new();
        let perto_a = world.spawn((Posicao(Vec3::ZERO), Colisor(Forma::Esfera { raio: 1.0 })));
        let perto_b = world.spawn((Posicao(Vec3::X), Colisor(Forma::Esfera { raio: 1.0 })));
        world.spawn((Posicao(Vec3::X * 50.0), Colisor(Forma::Esfera { raio: 1.0 })));

        let p = pares(&envelopes(&mut world));

        assert_eq!(p, vec![par_canonico(perto_a, perto_b)]);
    }
}
