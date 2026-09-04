//! Teste de determinismo cross-platform.
//!
//! Criterio de aceite da Fase 2, e a rede que protege as decisoes que a
//! sustentam: a ordem estavel de iteracao do ECS, o tick de cada sistema vindo
//! da ordem do schedule e nao de um contador atomico (D10), e a aplicacao dos
//! command buffers em ordem de insercao.
//!
//! O cenario e headless e fechado: semente fixa, nenhuma entrada externa,
//! numero fixo de passos. Ele roda igual em Windows e em Linux na matriz do CI,
//! e o hash resultante e comparado com um valor versionado. Uma divergencia
//! entre as duas plataformas aparece como falha em uma delas.
//!
//! # Quando este teste quebra
//!
//! Se a mudanca foi **intencional** — outro cenario, outra ordem de sistemas,
//! outra formula — incremente [`VERSAO_DO_CENARIO`] e atualize
//! [`HASH_ESPERADO`] com o valor que o proprio teste imprime.
//!
//! Se a mudanca **nao** foi intencional, o teste acabou de fazer o trabalho
//! dele: algo passou a depender de ordem instavel, de endereco de memoria, ou
//! de detalhe de plataforma.

use krateus_ecs::{Changed, Commands, Entity, Query, Res, ResMut, Schedule, World};
use krateus_simulation::{SimulationLoop, Time};

/// Muda junto com o cenario. Serve para que uma quebra deixe claro se o valor
/// esperado ficou velho de proposito ou nao.
const VERSAO_DO_CENARIO: u32 = 1;

/// Estado canonico apos [`PASSOS`] passos, para [`SEMENTE`].
///
/// Gerado pelo proprio teste. Ver o cabecalho do modulo antes de alterar.
const HASH_ESPERADO: u64 = 0x7fd4_9ff1_cfe5_0337;

const SEMENTE: u64 = 0x5eed_1234_abcd_0001;
const PASSOS: u32 = 300;
const ENTIDADES_INICIAIS: u32 = 500;
const HZ: u32 = 60;

// ------------------------------------------------------------------- hash --

/// FNV-1a de 64 bits.
///
/// Escolhido por ser especificado por inteiro — constantes, ordem de operacoes,
/// tudo — e portanto identico em qualquer plataforma e qualquer versao do
/// compilador. `DefaultHasher` nao serve aqui: a documentacao dele diz
/// explicitamente que o algoritmo pode mudar entre versoes do Rust, o que
/// transformaria uma atualizacao de toolchain numa falsa deteccao de
/// divergencia.
struct HashCanonico(u64);

impl HashCanonico {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIMO: u64 = 0x0000_0100_0000_01b3;

    fn novo() -> Self {
        Self(Self::OFFSET)
    }

    fn byte(&mut self, b: u8) {
        self.0 ^= u64::from(b);
        self.0 = self.0.wrapping_mul(Self::PRIMO);
    }

    /// Sempre little-endian, explicitamente.
    ///
    /// Windows e Linux em x86_64 sao os dois little-endian, entao hoje isto nao
    /// muda nada. Fixar a ordem agora evita que um alvo big-endian futuro
    /// produza divergencia silenciosa.
    fn u32(&mut self, v: u32) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    fn u64(&mut self, v: u64) {
        for b in v.to_le_bytes() {
            self.byte(b);
        }
    }

    fn i32(&mut self, v: i32) {
        // Reinterpretacao de bits em complemento de dois, que e o que se quer
        // hashear. `cast_unsigned` diria o mesmo com mais clareza, mas so
        // existe a partir do Rust 1.87 e o workspace declara 1.85.
        #[allow(clippy::cast_sign_loss)]
        self.u32(v as u32);
    }

    /// Hasheia os bits do float, com dois cuidados.
    ///
    /// `-0.0` e `0.0` sao iguais na comparacao e diferentes nos bits; sem
    /// normalizar, um sinal de zero acidental viraria divergencia. E `NaN` tem
    /// varias representacoes possiveis, entao qualquer `NaN` colapsa num unico
    /// marcador — se aparecer um, o problema e outro, e ha um teste separado
    /// para isso.
    fn f32(&mut self, v: f32) {
        if v.is_nan() {
            self.u32(0x7fc0_0000);
        } else if v == 0.0 {
            self.u32(0);
        } else {
            self.u32(v.to_bits());
        }
    }

    fn finalizar(self) -> u64 {
        self.0
    }
}

// -------------------------------------------------------------------- rng --

/// Gerador xorshift64*, implementado aqui de proposito.
///
/// Uma dependencia externa poderia mudar de algoritmo numa atualizacao menor e
/// quebrar o cenario sem que nada da engine tivesse mudado.
#[derive(Debug)]
struct Rng(u64);

impl Rng {
    fn nova(semente: u64) -> Self {
        // O estado nao pode ser zero: o xorshift ficaria preso nele.
        Self(if semente == 0 { 1 } else { semente })
    }

    fn proximo(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Float em `[0, 1)`, derivado so de inteiros e de uma divisao exata.
    fn unitario(&mut self) -> f32 {
        // 24 bits e a precisao da mantissa de f32; o divisor e potencia de dois,
        // entao a divisao e exata em IEEE-754 e nao depende de plataforma.
        let bits = (self.proximo() >> 40) as u32;
        bits as f32 / 16_777_216.0
    }

    fn faixa(&mut self, min: f32, max: f32) -> f32 {
        min + self.unitario() * (max - min)
    }
}

// ------------------------------------------------------------- componentes --

#[derive(Debug)]
struct Posicao(f32, f32, f32);
#[derive(Debug)]
struct Velocidade(f32, f32, f32);
#[derive(Debug)]
struct Saude(i32);
#[derive(Debug)]
struct Marca(u32);

#[derive(Debug, Default)]
struct Estatisticas {
    nascidas: u32,
    mortas: u32,
    movidas: u32,
}

// ---------------------------------------------------------------- sistemas --

fn gravidade(mut q: Query<&mut Velocidade>, tempo: Res<Time>) {
    let dt = tempo.delta_seconds();
    for v in q.iter() {
        v.1 -= 9.81 * dt;
    }
}

fn integrar(mut q: Query<(&mut Posicao, &Velocidade)>, tempo: Res<Time>) {
    let dt = tempo.delta_seconds();
    for (p, v) in q.iter() {
        p.0 += v.0 * dt;
        p.1 += v.1 * dt;
        p.2 += v.2 * dt;
    }
}

fn quicar(mut q: Query<(&mut Posicao, &mut Velocidade)>) {
    for (p, v) in q.iter() {
        if p.1 < 0.0 {
            p.1 = -p.1;
            v.1 = -v.1 * 0.8;
        }
    }
}

fn envelhecer(mut q: Query<&mut Saude>) {
    for s in q.iter() {
        s.0 -= 1;
    }
}

fn ceifar(mut q: Query<(Entity, &Saude)>, cmds: &mut Commands, mut est: ResMut<Estatisticas>) {
    for (e, s) in q.iter() {
        if s.0 <= 0 {
            cmds.despawn(e);
            est.mortas += 1;
        }
    }
}

/// Conta quantas entidades tiveram a posicao exposta para escrita desde a
/// execucao anterior deste sistema.
///
/// Existe para que o hash dependa da deteccao de mudanca — e, por tabela, da
/// atribuicao de ticks pelo schedule.
fn contar_movidas(mut q: Query<&Posicao, Changed<Posicao>>, mut est: ResMut<Estatisticas>) {
    est.movidas = est.movidas.wrapping_add(u32::try_from(q.iter().count()).unwrap_or(u32::MAX));
}

fn semear(
    mut rng: ResMut<Rng>,
    tempo: Res<Time>,
    cmds: &mut Commands,
    mut est: ResMut<Estatisticas>,
) {
    // A cada cinco passos nascem tres entidades. O gatilho e a contagem de
    // passos, nunca tempo de relogio.
    if tempo.tick % 5 != 0 {
        return;
    }
    for _ in 0..3 {
        let marca = (rng.proximo() >> 32) as u32;
        let p = (rng.faixa(-10.0, 10.0), rng.faixa(0.0, 20.0), rng.faixa(-10.0, 10.0));
        let v = (rng.faixa(-2.0, 2.0), rng.faixa(-1.0, 1.0), rng.faixa(-2.0, 2.0));
        let vida = 30 + (rng.proximo() % 90) as i32;

        cmds.spawn((Posicao(p.0, p.1, p.2), Velocidade(v.0, v.1, v.2), Saude(vida), Marca(marca)));
        est.nascidas += 1;
    }
}

// ----------------------------------------------------------------- cenario --

fn montar() -> (World, Schedule) {
    let mut world = World::new();
    let mut rng = Rng::nova(SEMENTE);

    for i in 0..ENTIDADES_INICIAIS {
        let p = (rng.faixa(-50.0, 50.0), rng.faixa(0.0, 30.0), rng.faixa(-50.0, 50.0));
        let v = (rng.faixa(-5.0, 5.0), rng.faixa(-2.0, 2.0), rng.faixa(-5.0, 5.0));
        let vida = 50 + (rng.proximo() % 200) as i32;
        world.spawn((Posicao(p.0, p.1, p.2), Velocidade(v.0, v.1, v.2), Saude(vida), Marca(i)));
    }

    world.insert_resource(rng);
    world.insert_resource(Estatisticas::default());

    let mut schedule = Schedule::new();
    schedule.add_stage("fisica").add_stage("vida").add_stage("mundo");

    schedule.add_system("fisica", gravidade);
    schedule.add_system("fisica", integrar);
    schedule.add_system("fisica", quicar);
    schedule.add_system("vida", envelhecer);
    schedule.add_system("vida", ceifar);
    schedule.add_system("mundo", contar_movidas);
    schedule.add_system("mundo", semear);

    (world, schedule)
}

fn rodar(paralelo: bool) -> World {
    let (mut world, mut schedule) = montar();
    let mut laco = SimulationLoop::new(HZ);
    laco.initialize(&mut world, &mut schedule);

    // Alimenta exatamente um passo por iteracao: sem sobra, sem descarte, sem
    // dependencia de quanto tempo real a maquina levou.
    let passo = laco.timestep();
    let pool = krateus_core::jobs::JobPool::new(if paralelo { 4 } else { 0 });

    for _ in 0..PASSOS {
        let avanco = if paralelo {
            laco.advance_parallel(passo, &mut world, &mut schedule, &pool)
        } else {
            laco.advance(passo, &mut world, &mut schedule)
        };
        assert_eq!(avanco.passos, 1, "o cenario precisa avancar exatamente um passo por iteracao");
        assert!(!avanco.descartou_tempo);
    }

    world
}

/// Hash canonico do estado do mundo.
///
/// A ordem e o ponto: as entidades sao ordenadas por identificador antes de
/// entrar no hash. Percorrer os archetypes na ordem em que estao produziria um
/// valor que depende de quem foi criado e destruido quando — informacao que o
/// estado logico nao carrega.
fn hash_do_estado(world: &World) -> u64 {
    let mut entidades: Vec<Entity> = world.entities().iter().collect();
    entidades.sort_unstable();

    let mut h = HashCanonico::novo();
    h.u32(VERSAO_DO_CENARIO);
    h.u64(entidades.len() as u64);

    for e in entidades {
        h.u32(e.index());
        h.u32(e.generation().get());

        // Marcador de presenca antes de cada componente: sem ele, "sem Posicao"
        // e "Posicao zerada" colidiriam.
        match world.get::<Posicao>(e) {
            Some(p) => {
                h.byte(1);
                h.f32(p.0);
                h.f32(p.1);
                h.f32(p.2);
            }
            None => h.byte(0),
        }
        match world.get::<Velocidade>(e) {
            Some(v) => {
                h.byte(1);
                h.f32(v.0);
                h.f32(v.1);
                h.f32(v.2);
            }
            None => h.byte(0),
        }
        match world.get::<Saude>(e) {
            Some(s) => {
                h.byte(1);
                h.i32(s.0);
            }
            None => h.byte(0),
        }
        match world.get::<Marca>(e) {
            Some(m) => {
                h.byte(1);
                h.u32(m.0);
            }
            None => h.byte(0),
        }
    }

    let est = world.resource::<Estatisticas>();
    h.u32(est.nascidas);
    h.u32(est.mortas);
    h.u32(est.movidas);

    h.finalizar()
}

// ------------------------------------------------------------------ testes --

#[test]
fn hash_bate_com_o_valor_versionado() {
    let obtido = hash_do_estado(&rodar(false));

    assert_eq!(
        obtido, HASH_ESPERADO,
        "\no estado apos {PASSOS} passos divergiu do valor versionado.\n\
         \n\
         obtido:   0x{obtido:016x}\n\
         esperado: 0x{HASH_ESPERADO:016x}\n\
         \n\
         Se a mudanca foi intencional, incremente VERSAO_DO_CENARIO e troque\n\
         HASH_ESPERADO pelo valor obtido. Se nao foi, algo passou a depender de\n\
         ordem instavel, de endereco de memoria ou de detalhe de plataforma.\n"
    );
}

#[test]
fn a_mesma_semente_produz_o_mesmo_estado() {
    // Independente do valor versionado: duas execucoes no mesmo processo
    // precisam coincidir. Falha aqui aponta para estado global ou ordem
    // instavel dentro de uma unica plataforma.
    assert_eq!(hash_do_estado(&rodar(false)), hash_do_estado(&rodar(false)));
}

#[test]
fn execucao_paralela_produz_o_mesmo_estado_da_sequencial() {
    // O paralelismo nao pode mudar o resultado: sistemas de uma mesma subetapa
    // tem acessos disjuntos, e os comandos sao aplicados em ordem de insercao.
    assert_eq!(hash_do_estado(&rodar(true)), hash_do_estado(&rodar(false)));
}

#[test]
fn o_cenario_exercita_o_que_deveria() {
    // Um cenario que nao cria, nao destroi e nao detecta mudanca passaria o
    // teste de hash sem proteger nada.
    let world = rodar(false);
    let est = world.resource::<Estatisticas>();

    assert!(est.nascidas > 100, "nasceram poucas: {}", est.nascidas);
    assert!(est.mortas > 100, "morreram poucas: {}", est.mortas);
    assert!(est.movidas > 0, "a deteccao de mudanca nao foi exercitada");
    assert!(!world.is_empty(), "o mundo terminou vazio");
}

#[test]
fn nenhum_valor_vira_nan_ou_infinito() {
    // O hash colapsa NaN num marcador unico, o que esconderia uma divergencia
    // numerica real. Este teste garante que o cenario nunca chega la.
    let world = rodar(false);
    let entidades: Vec<Entity> = world.entities().iter().collect();

    for e in entidades {
        if let Some(p) = world.get::<Posicao>(e) {
            assert!(p.0.is_finite() && p.1.is_finite() && p.2.is_finite(), "posicao nao finita");
        }
        if let Some(v) = world.get::<Velocidade>(e) {
            assert!(v.0.is_finite() && v.1.is_finite() && v.2.is_finite(), "velocidade nao finita");
        }
    }

    assert!(!world.is_empty());
}

#[test]
fn hash_reage_a_qualquer_diferenca_de_estado() {
    // Um hash que nao distingue estados diferentes passaria sempre.
    let mut a = rodar(false);
    let base = hash_do_estado(&a);

    let alvo = a.entities().iter().next().expect("mundo nao vazio");
    a.get_mut::<Posicao>(alvo).expect("entidade com posicao").0 += 1.0;

    assert_ne!(hash_do_estado(&a), base);
}

#[test]
fn zero_negativo_nao_diverge_de_zero() {
    let mut a = HashCanonico::novo();
    a.f32(0.0);
    let mut b = HashCanonico::novo();
    b.f32(-0.0);

    assert_eq!(a.finalizar(), b.finalizar(), "o sinal do zero nao pode virar divergencia");
}
