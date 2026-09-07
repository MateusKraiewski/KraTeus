//! Integrador semi-implicito, com gravidade, forcas e impulsos.
//!
//! # Por que semi-implicito
//!
//! Euler explicito calcula a posicao com a velocidade **antiga**; o
//! semi-implicito atualiza a velocidade primeiro e usa a nova. A diferenca e uma
//! linha de codigo e muda o comportamento qualitativo: o semi-implicito e
//! simpletico, o que na pratica significa que a energia de um sistema oscilante
//! fica **limitada** em vez de crescer sem parar.
//!
//! Isso importa porque quase tudo em fisica de jogo oscila — mola, suspensao,
//! contato com penetracao. Com Euler explicito, uma pilha de caixas ganha
//! energia sozinha e explode. Ha teste comparando os dois neste modulo, para que
//! a escolha nao pareca arbitraria.
//!
//! # Posicao propria, e nao o `Transform` do renderer
//!
//! A fisica define [`Posicao`] em vez de reusar o `Transform` de
//! `krateus-render`. Nao e so evitar a dependencia na direcao errada: sao
//! conceitos diferentes. O renderer precisa de translacao, rotacao **e escala**;
//! a fisica nao simula escala, e um corpo rigido tem orientacao ligada a inercia,
//! nao a apresentacao. Um sistema de sincronizacao liga os dois quando houver
//! quem precise — sem consumidor, escolher agora seria adivinhar.

use glam::Vec3;
use krateus_core::time::Time;
use krateus_ecs::{Query, Res};

/// Posicao de um corpo no mundo.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Posicao(pub Vec3);

/// Velocidade linear de um corpo.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Velocidade(pub Vec3);

/// Inverso da massa, em 1/kg.
///
/// Guardar o inverso, e nao a massa, tem duas razoes. A divisao por massa
/// aparece em todo passo de integracao e em toda resolucao de contato, e
/// guardar `1/m` a transforma em multiplicacao. E massa infinita — um corpo
/// estatico, o chao — vira `0.0`, que e um numero comum, em vez de um caso
/// especial espalhado por toda parte.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MassaInversa(pub f32);

impl Default for MassaInversa {
    fn default() -> Self {
        Self(1.0)
    }
}

impl MassaInversa {
    /// Corpo que nao se move, por mais forca que receba.
    pub const ESTATICO: Self = Self(0.0);

    /// Constroi a partir da massa em kg.
    ///
    /// Massa nao positiva vira estatico: e o que "massa infinita" significa, e
    /// evita produzir infinito ou `NaN` no primeiro passo.
    #[must_use]
    pub fn de_massa(kg: f32) -> Self {
        if kg > 0.0 { Self(1.0 / kg) } else { Self::ESTATICO }
    }

    /// Massa em kg, ou `None` para um corpo estatico.
    #[must_use]
    pub fn massa(self) -> Option<f32> {
        if self.0 > 0.0 { Some(1.0 / self.0) } else { None }
    }

    /// Indica se o corpo e imovel.
    #[must_use]
    pub fn e_estatico(self) -> bool {
        self.0 <= 0.0
    }
}

/// Forcas somadas desde o ultimo passo.
///
/// Zerado ao fim de cada integracao: uma forca vale por um passo. Quem quer
/// forca continua a reaplica, o que e o mesmo contrato de qualquer motor de
/// fisica e evita o bug de uma forca esquecida empurrando para sempre.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct AcumuladorDeForca(pub Vec3);

impl AcumuladorDeForca {
    /// Acrescenta uma forca, em newtons.
    pub fn aplicar(&mut self, forca: Vec3) {
        self.0 += forca;
    }
}

/// Aceleracao gravitacional do mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Gravidade(pub Vec3);

impl Default for Gravidade {
    fn default() -> Self {
        Self(Vec3::new(0.0, -crate::trajetoria::G_TERRA, 0.0))
    }
}

/// Aplica um impulso, mudando a velocidade de uma vez.
///
/// Impulso e forca integrada no tempo: um tiro, uma explosao, um pulo. Diferente
/// de forca, ele nao espera o proximo passo — e por isso nao passa pelo
/// acumulador.
pub fn aplicar_impulso(velocidade: &mut Velocidade, massa_inv: MassaInversa, impulso: Vec3) {
    velocidade.0 += impulso * massa_inv.0;
}

/// Integra um corpo por `dt`.
///
/// Exposto separadamente do sistema para que uma ferramenta — o preview de
/// trajetoria do editor, por exemplo — possa simular sem montar um `World`.
pub fn integrar_um_passo(
    posicao: &mut Posicao,
    velocidade: &mut Velocidade,
    forca: &mut AcumuladorDeForca,
    massa_inv: MassaInversa,
    gravidade: Vec3,
    dt: f32,
) {
    if massa_inv.e_estatico() {
        // Um corpo estatico nao acumula velocidade nem consome forca; zerar o
        // acumulador ainda assim evita que uma forca aplicada por engano fique
        // guardada para quando ele deixar de ser estatico.
        forca.0 = Vec3::ZERO;
        return;
    }

    // Gravidade e aceleracao, nao forca: ela independe da massa. Somar `m·g` ao
    // acumulador para dividir por `m` logo em seguida so introduziria erro.
    let aceleracao = forca.0 * massa_inv.0 + gravidade;

    // Semi-implicito: velocidade primeiro, posicao com a velocidade nova.
    velocidade.0 += aceleracao * dt;
    posicao.0 += velocidade.0 * dt;

    forca.0 = Vec3::ZERO;
}

/// Sistema que integra todos os corpos do mundo.
pub fn sistema_integrar(
    mut corpos: Query<(&mut Posicao, &mut Velocidade, &mut AcumuladorDeForca, &MassaInversa)>,
    tempo: Res<Time>,
    gravidade: Res<Gravidade>,
) {
    let dt = tempo.delta_seconds();
    let g = gravidade.0;

    for (posicao, velocidade, forca, massa_inv) in corpos.iter() {
        integrar_um_passo(posicao, velocidade, forca, *massa_inv, g, dt);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trajetoria::G_TERRA;

    const DT: f32 = 1.0 / 60.0;

    struct Corpo {
        posicao: Posicao,
        velocidade: Velocidade,
        forca: AcumuladorDeForca,
        massa_inv: MassaInversa,
    }

    impl Corpo {
        fn de_massa(kg: f32) -> Self {
            Self {
                posicao: Posicao::default(),
                velocidade: Velocidade::default(),
                forca: AcumuladorDeForca::default(),
                massa_inv: MassaInversa::de_massa(kg),
            }
        }

        fn passo(&mut self, gravidade: Vec3, dt: f32) {
            integrar_um_passo(
                &mut self.posicao,
                &mut self.velocidade,
                &mut self.forca,
                self.massa_inv,
                gravidade,
                dt,
            );
        }
    }

    // --------------------------------------------------------------- massa --

    #[test]
    fn massa_inversa_ida_e_volta() {
        let m = MassaInversa::de_massa(4.0);
        assert!((m.0 - 0.25).abs() < 1e-6);
        assert!((m.massa().expect("nao estatico") - 4.0).abs() < 1e-4);
    }

    #[test]
    fn massa_nao_positiva_vira_estatico() {
        for kg in [0.0, -1.0, -0.0] {
            let m = MassaInversa::de_massa(kg);
            assert!(m.e_estatico(), "massa {kg} deveria virar estatico");
            assert_eq!(m.massa(), None);
            assert!(m.0.is_finite(), "estatico nao pode virar infinito");
        }
    }

    // ---------------------------------------------------------- queda livre --

    #[test]
    fn corpo_cai_sob_gravidade() {
        let mut c = Corpo::de_massa(1.0);
        let g = Vec3::new(0.0, -G_TERRA, 0.0);

        for _ in 0..60 {
            c.passo(g, DT);
        }

        // Um segundo de queda: a velocidade e praticamente `g`.
        assert!((c.velocidade.0.y + G_TERRA).abs() < 0.2, "v = {}", c.velocidade.0.y);
        assert!(c.posicao.0.y < -4.0, "deveria ter caido, y = {}", c.posicao.0.y);
    }

    #[test]
    fn a_massa_nao_altera_a_queda() {
        // Galileu: sem arrasto, corpos de massas diferentes caem igual.
        let g = Vec3::new(0.0, -G_TERRA, 0.0);
        let mut leve = Corpo::de_massa(0.1);
        let mut pesado = Corpo::de_massa(1000.0);

        for _ in 0..120 {
            leve.passo(g, DT);
            pesado.passo(g, DT);
        }

        assert_eq!(leve.posicao.0, pesado.posicao.0);
    }

    #[test]
    fn corpo_estatico_nao_se_move() {
        let mut c = Corpo::de_massa(0.0);
        c.forca.aplicar(Vec3::new(1_000.0, 1_000.0, 1_000.0));

        for _ in 0..100 {
            c.passo(Vec3::new(0.0, -G_TERRA, 0.0), DT);
        }

        assert_eq!(c.posicao.0, Vec3::ZERO);
        assert_eq!(c.velocidade.0, Vec3::ZERO);
    }

    // --------------------------------------------------------------- forcas --

    #[test]
    fn forca_acelera_na_proporcao_inversa_da_massa() {
        let mut leve = Corpo::de_massa(1.0);
        let mut pesado = Corpo::de_massa(2.0);

        leve.forca.aplicar(Vec3::X * 10.0);
        pesado.forca.aplicar(Vec3::X * 10.0);
        leve.passo(Vec3::ZERO, DT);
        pesado.passo(Vec3::ZERO, DT);

        assert!((leve.velocidade.0.x - 2.0 * pesado.velocidade.0.x).abs() < 1e-5);
    }

    #[test]
    fn a_forca_vale_por_um_passo_so() {
        // O acumulador zera; quem quer forca continua reaplica. Sem isso, uma
        // forca esquecida empurraria para sempre.
        let mut c = Corpo::de_massa(1.0);
        c.forca.aplicar(Vec3::X * 10.0);

        c.passo(Vec3::ZERO, DT);
        let depois_de_um = c.velocidade.0.x;

        c.passo(Vec3::ZERO, DT);
        assert_eq!(c.velocidade.0.x, depois_de_um, "a forca nao pode persistir");
        assert_eq!(c.forca.0, Vec3::ZERO);
    }

    #[test]
    fn forcas_do_mesmo_passo_se_somam() {
        let mut c = Corpo::de_massa(1.0);
        c.forca.aplicar(Vec3::X * 3.0);
        c.forca.aplicar(Vec3::Y * 4.0);
        c.passo(Vec3::ZERO, DT);

        assert!((c.velocidade.0.x - 3.0 * DT).abs() < 1e-6);
        assert!((c.velocidade.0.y - 4.0 * DT).abs() < 1e-6);
    }

    #[test]
    fn forca_em_corpo_estatico_nao_fica_guardada() {
        let mut c = Corpo::de_massa(0.0);
        c.forca.aplicar(Vec3::X * 100.0);
        c.passo(Vec3::ZERO, DT);

        assert_eq!(c.forca.0, Vec3::ZERO, "a forca precisa ser descartada, nao acumulada");
    }

    // -------------------------------------------------------------- impulso --

    #[test]
    fn impulso_muda_a_velocidade_na_hora() {
        let mut v = Velocidade::default();
        aplicar_impulso(&mut v, MassaInversa::de_massa(2.0), Vec3::Y * 10.0);

        assert!((v.0.y - 5.0).abs() < 1e-6, "impulso/massa = 10/2");
    }

    #[test]
    fn impulso_nao_move_corpo_estatico() {
        let mut v = Velocidade::default();
        aplicar_impulso(&mut v, MassaInversa::ESTATICO, Vec3::Y * 1_000.0);

        assert_eq!(v.0, Vec3::ZERO);
    }

    // ------------------------------------------------------------- energia --

    /// Energia total de uma mola: cinetica mais potencial elastica.
    fn energia_da_mola(pos: f32, vel: f32, k: f32, massa: f32) -> f32 {
        0.5 * massa * vel * vel + 0.5 * k * pos * pos
    }

    /// Uma mola de 50 N/m e 1 kg, esticada 1 m, integrada por `passos`.
    ///
    /// Devolve a energia total apos cada passo.
    fn oscilar(dt: f32, passos: usize) -> Vec<f32> {
        const K: f32 = 50.0;

        let mut c = Corpo::de_massa(1.0);
        c.posicao.0 = Vec3::X;

        (0..passos)
            .map(|_| {
                c.forca.aplicar(Vec3::X * (-K * c.posicao.0.x));
                c.passo(Vec3::ZERO, dt);
                energia_da_mola(c.posicao.0.x, c.velocidade.0.x, K, 1.0)
            })
            .collect()
    }

    /// Menor e maior valor de uma serie.
    fn envelope(serie: &[f32]) -> (f32, f32) {
        serie.iter().fold((f32::MAX, f32::MIN), |(mn, mx), &v| (mn.min(v), mx.max(v)))
    }

    #[test]
    fn a_energia_de_um_oscilador_nao_deriva() {
        // Criterio de aceite da Fase 5, e o teste que justifica a escolha do
        // integrador.
        //
        // O semi-implicito **nao** conserva energia exatamente: ela oscila. O
        // que ele garante e que o envelope dessa oscilacao nao cresce. E essa a
        // propriedade simpletica — erro limitado, em vez de erro acumulado — e e
        // ela que impede uma pilha de caixas de explodir sozinha.
        let e = oscilar(DT, 6_000); // 100 s a 60 Hz
        let (min_ini, max_ini) = envelope(&e[..600]);
        let (min_fim, max_fim) = envelope(&e[e.len() - 600..]);

        assert!(
            ((max_fim - max_ini) / max_ini).abs() < 1e-3,
            "o topo do envelope derivou: {max_ini} -> {max_fim}"
        );
        assert!(
            ((min_fim - min_ini) / min_ini).abs() < 1e-3,
            "o fundo do envelope derivou: {min_ini} -> {min_fim}"
        );

        // A oscilacao em si e da ordem de omega*dt — cerca de 12% aqui. Fixar
        // uma tolerancia menor seria afirmar uma precisao que o integrador de
        // primeira ordem nao entrega, e o teste passaria a mentir.
        let inicial = 0.5 * 50.0;
        let (min_total, max_total) = envelope(&e);
        assert!((max_total - min_total) / inicial < 0.15);
    }

    #[test]
    fn o_erro_de_energia_cai_junto_com_o_passo() {
        // Primeira ordem: metade do passo, metade da oscilacao. Trocar o
        // integrador por um de ordem maior — Verlet, RK4 — muda esta razao, e e
        // por aqui que a troca aparece.
        let amplitude = |dt: f32, passos: usize| {
            let (mn, mx) = envelope(&oscilar(dt, passos));
            mx - mn
        };

        let razao = amplitude(DT, 6_000) / amplitude(DT / 2.0, 12_000);
        assert!((1.8..2.2).contains(&razao), "esperado ~2, medido {razao}");
    }

    #[test]
    fn euler_explicito_ganharia_energia_no_mesmo_caso() {
        // Documenta por que a escolha nao e arbitraria. Mesmo sistema, mesma
        // gravidade, mesmo passo — so a posicao calculada com a velocidade
        // *antiga*. A energia cresce cerca de seis ordens de grandeza em mil
        // passos, que e a pilha de caixas explodindo.
        let k = 50.0_f32;
        let (mut pos, mut vel) = (1.0_f32, 0.0_f32);
        let inicial = energia_da_mola(pos, vel, k, 1.0);

        for _ in 0..1_000 {
            let aceleracao = -k * pos;
            pos += vel * DT; // a velocidade antiga, e nao a nova
            vel += aceleracao * DT;
        }

        let fim = energia_da_mola(pos, vel, k, 1.0);
        assert!(fim > inicial * 100.0, "de {inicial:.2} para {fim:.2}");
    }

    // --------------------------------------------------------- determinismo --

    #[test]
    fn a_mesma_entrada_produz_o_mesmo_resultado() {
        let simular = || {
            let mut c = Corpo::de_massa(2.5);
            let g = Vec3::new(0.0, -G_TERRA, 0.0);
            for i in 0..500 {
                c.forca.aplicar(Vec3::new(i as f32 * 0.01, 0.0, -0.5));
                c.passo(g, DT);
            }
            (c.posicao.0.to_array(), c.velocidade.0.to_array())
        };

        assert_eq!(simular(), simular());
    }

    // ------------------------------------------------------ como sistema ECS --

    /// Passo de 60 Hz como o laco de simulacao o entrega.
    fn tempo_de_60_hz() -> Time {
        let delta = std::time::Duration::from_nanos(16_666_666);
        Time { tick: 1, delta, elapsed: delta }
    }

    #[test]
    fn o_sistema_faz_o_mesmo_que_a_funcao() {
        use krateus_ecs::{Schedule, World};

        const XS: [f32; 5] = [0.0, 1.0, 2.0, 3.0, 4.0];

        let mut world = World::new();
        world.insert_resource(Gravidade::default());
        world.insert_resource(tempo_de_60_hz());

        let corpos: Vec<_> = XS
            .into_iter()
            .map(|x| {
                world.spawn((
                    Posicao(Vec3::new(x, 10.0, 0.0)),
                    Velocidade::default(),
                    AcumuladorDeForca::default(),
                    MassaInversa::de_massa(1.0),
                ))
            })
            .collect();

        let mut schedule = Schedule::new();
        schedule.add_stage("fisica");
        schedule.add_system("fisica", sistema_integrar);
        schedule.initialize(&mut world);

        for _ in 0..60 {
            schedule.run(&mut world);
        }

        // A referencia e a mesma funcao, chamada fora do mundo. Comparar contra
        // ela — e nao contra um limiar escolhido a olho — verifica o que o
        // sistema deve fazer: aplicar a integracao, sem mudar a conta.
        let dt = tempo_de_60_hz().delta_seconds();
        let mut referencia = Corpo::de_massa(1.0);
        referencia.posicao.0 = Vec3::new(0.0, 10.0, 0.0);
        for _ in 0..60 {
            referencia.passo(Gravidade::default().0, dt);
        }

        for (e, x) in corpos.into_iter().zip(XS) {
            let p = world.get::<Posicao>(e).expect("corpo vivo");
            assert_eq!(p.0.y, referencia.posicao.0.y, "queda diferente da referencia");
            assert_eq!(p.0.x, x, "sem forca horizontal, x nao muda");
        }

        // E, para que o teste nao passe por um mundo parado: um segundo de
        // queda livre sao pouco menos de 5 m.
        assert!(referencia.posicao.0.y < 5.1, "y = {}", referencia.posicao.0.y);
        assert!(referencia.posicao.0.y > 4.9, "y = {}", referencia.posicao.0.y);
    }

    #[test]
    fn corpos_estaticos_convivem_com_dinamicos() {
        use krateus_ecs::{Schedule, World};

        let mut world = World::new();
        world.insert_resource(Gravidade::default());
        world.insert_resource(tempo_de_60_hz());

        let chao = world.spawn((
            Posicao(Vec3::ZERO),
            Velocidade::default(),
            AcumuladorDeForca::default(),
            MassaInversa::ESTATICO,
        ));
        let caixa = world.spawn((
            Posicao(Vec3::Y * 10.0),
            Velocidade::default(),
            AcumuladorDeForca::default(),
            MassaInversa::de_massa(1.0),
        ));

        let mut schedule = Schedule::new();
        schedule.add_stage("fisica");
        schedule.add_system("fisica", sistema_integrar);
        schedule.initialize(&mut world);

        for _ in 0..30 {
            schedule.run(&mut world);
        }

        assert_eq!(world.get::<Posicao>(chao).expect("vivo").0, Vec3::ZERO);
        assert!(world.get::<Posicao>(caixa).expect("vivo").0.y < 10.0);
    }
}
