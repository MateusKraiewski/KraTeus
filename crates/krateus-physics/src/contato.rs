//! Narrowphase: dado um par candidato, existe contato e qual e a sua geometria.
//!
//! A broadphase entrega pares que **podem** estar em contato. Aqui se decide se
//! estao, e em caso afirmativo se produz normal, profundidade e pontos. Nada
//! alem disso: resolver o contato — impulso, atrito, restituicao, correcao de
//! posicao — e do solver, que ainda nao existe.
//!
//! ```text
//! broadphase → pares candidatos → narrowphase → Option<Contato> → solver
//! ```
//!
//! # Convencoes, e por que elas importam
//!
//! Estao na [D15](../../../docs/DECISOES.md), e sao quatro:
//!
//! 1. a normal aponta **de A para B** e e unitaria;
//! 2. a profundidade e **sempre ≥ 0** — tangencia exata e contato de
//!    profundidade zero;
//! 3. os pontos ficam na **superficie de A**, e sao os pontos de A mais fundos
//!    dentro de B;
//! 4. o ponto correspondente em B e `ponto − normal * profundidade`.
//!
//! O sinal da quarta regra e `−`, e vale conferir com duas esferas de raio 1 em
//! `(0,0,0)` e `(1,5, 0, 0)`: a normal e `(1,0,0)`, a profundidade `0,5`, o ponto
//! de A e `(1,0,0)` e o de B e `(0,5, 0, 0)`. A diferenca e `normal *
//! profundidade`, entao B esta a `ponto − normal * profundidade`. Com `+` o
//! ponto cairia do outro lado do corpo.
//!
//! # Estado
//!
//! Este modulo esta sendo construido em tres incrementos. **Este e o primeiro**,
//! e cobre os grupos A e B:
//!
//! | Par | Estado |
//! |---|---|
//! | Esfera × Esfera | ✅ nucleo inflado |
//! | Esfera × Capsula | ✅ nucleo inflado |
//! | Capsula × Capsula | ✅ nucleo inflado |
//! | Esfera × Plano | ✅ semiespaco |
//! | Capsula × Plano | ✅ semiespaco |
//! | Caixa × Plano | ✅ semiespaco |
//! | Plano × Plano | ✅ sem contato finito |
//! | Esfera × Caixa | ✅ ponto mais proximo na OBB |
//! | Caixa × Capsula | ✅ distancia fechada, e SAT quando atravessa |
//! | Caixa × Caixa | ⬜ grupo D |
//!
//! Nao ha despacho geral por [`Forma`](crate::Forma) ainda, **de proposito**: um
//! despacho que devolvesse `None` para os pares que faltam seria silenciosamente
//! incompleto, indistinguivel de "nao ha contato". Ele entra no ultimo
//! incremento, quando a matriz estiver inteira.
//!
//! # Nucleo inflado
//!
//! Esfera e capsula sao a mesma coisa: um ponto ou um segmento engordado por um
//! raio. Os tres pares entre elas reduzem a **um** algoritmo — os pontos mais
//! proximos entre dois segmentos, com a esfera como segmento de comprimento zero
//! — seguido de subtrair a soma dos raios. Tres celulas da matriz, uma
//! implementacao, um conjunto de casos degenerados.
//!
//! # Determinismo
//!
//! Nenhuma funcao transcendental, e raiz quadrada apenas onde a distancia
//! precisa ser um comprimento. Todo desempate e explicito e esta comentado onde
//! acontece. Os pontos do manifold saem em ordem canonica, para que
//! `contato(A, B)` e `contato(B, A)` invertido sejam comparaveis por igualdade.

use glam::{Mat3, Quat, Vec3};

/// Posicao e orientacao de um corpo.
///
/// Existe porque a narrowphase trabalha sempre com **dois** corpos, e passar
/// quatro valores soltos por chamada tornaria as assinaturas ilegiveis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    /// Posicao no mundo.
    pub posicao: Vec3,
    /// Orientacao.
    pub rotacao: Quat,
}

impl Pose {
    /// Pose com posicao e orientacao.
    #[must_use]
    pub const fn nova(posicao: Vec3, rotacao: Quat) -> Self {
        Self { posicao, rotacao }
    }

    /// Pose sem rotacao.
    ///
    /// E a forma comum hoje: a fisica ainda nao tem dinamica angular, entao os
    /// corpos do mundo estao todos assim.
    #[must_use]
    pub const fn em(posicao: Vec3) -> Self {
        Self { posicao, rotacao: Quat::IDENTITY }
    }
}

/// Pontos maximos num manifold.
///
/// Quatro porque e o que a face de uma caixa apoiada exige, e nada na matriz de
/// formas exige mais. Ver [D15](../../../docs/DECISOES.md).
pub const MAX_PONTOS: usize = 4;

// -------------------------------------------------------------- tolerancias --

/// Abaixo disto, um segmento e tratado como ponto.
///
/// E um comprimento **ao quadrado**: `1e-12` corresponde a um segmento de 1e-6
/// unidades — um micrometro, num mundo em metros. Uma capsula de meia altura
/// menor que isso e uma esfera, e dividir pelo comprimento amplificaria ruido em
/// vez de informacao.
const EPS_COMPRIMENTO_QUADRADO: f32 = 1e-12;

/// Limiar relativo para considerar dois segmentos paralelos.
///
/// O denominador do sistema e `a·e − b² = a·e·sin²θ`, entao comparar contra
/// `EPS·a·e` e comparar `sin²θ` contra `EPS` — um teste **livre de escala**, que
/// nao muda de significado com o tamanho dos corpos. Com `1e-6`, o limite fica
/// em `θ ≈ 1e-3 rad`, cerca de 0,057°: abaixo disso a solucao do sistema perde
/// precisao mais rapido do que a resposta paralela erra.
const EPS_PARALELO: f32 = 1e-6;

/// Abaixo disto, a direcao entre os pontos mais proximos nao tem significado.
///
/// Tambem um comprimento ao quadrado. Coordenadas de magnitude ~1 carregam erro
/// absoluto de ~1e-7; se a distancia entre os pontos e da mesma ordem, a direcao
/// normalizada tem erro relativo da ordem de dezenas por cento. Nesse regime o
/// codigo escolhe uma direcao fixa em vez de normalizar ruido.
const EPS_DIRECAO_QUADRADA: f32 = 1e-12;

/// Diferenca de profundidade que ainda conta como "mesmo plano de contato".
///
/// Nao e um epsilon de igualdade de float: e a afirmacao geometrica de que a
/// face esta apoiada. Numa caixa de meia extensao 1, uma diferenca de 1e-4 entre
/// vertices corresponde a uma inclinacao de ~5e-5 rad. Acima disso a caixa esta
/// inclinada e apoia um vertice, nao uma face — e o manifold precisa refletir
/// isso, sob pena de inventar um apoio que nao existe.
const TOLERANCIA_COPLANAR: f32 = 1e-4;

/// Distancia minima, no mundo, entre dois pontos distintos de um manifold.
///
/// Dois pontos mais proximos que isto sao o mesmo ponto para qualquer efeito, e
/// entrega-los separados daria ao solver duas restricoes quase identicas.
const EPS_SEPARACAO_PONTOS: f32 = 1e-4;

// ------------------------------------------------------------------ contato --

/// Um contato entre dois corpos.
///
/// Os campos sao privados porque as invariantes sao o contrato: normal unitaria,
/// profundidade nao negativa, entre um e [`MAX_PONTOS`] pontos, em ordem
/// canonica.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Contato {
    normal: Vec3,
    profundidade: f32,
    pontos: [Vec3; MAX_PONTOS],
    quantidade: u8,
}

impl Contato {
    /// Constroi um contato.
    ///
    /// Os pontos sao reordenados canonicamente — por x, depois y, depois z. Sem
    /// isso, o mesmo contato calculado nas duas ordens poderia sair com os
    /// pontos trocados de lugar, e a comparacao entre eles deixaria de ser
    /// igualdade.
    ///
    /// # Panics
    ///
    /// Se `pontos` estiver vazio ou tiver mais de [`MAX_PONTOS`] elementos.
    #[must_use]
    pub fn novo(normal: Vec3, profundidade: f32, pontos: &[Vec3]) -> Self {
        assert!(!pontos.is_empty(), "um contato precisa de ao menos um ponto");
        assert!(pontos.len() <= MAX_PONTOS, "o manifold tem no maximo {MAX_PONTOS} pontos");
        debug_assert!(profundidade >= 0.0, "a profundidade e nao negativa (D15)");
        debug_assert!(
            (normal.length_squared() - 1.0).abs() < 1e-4,
            "a normal precisa ser unitaria (D15)"
        );

        let mut buffer = [Vec3::ZERO; MAX_PONTOS];
        buffer[..pontos.len()].copy_from_slice(pontos);
        buffer[..pontos.len()].sort_by(ordem_canonica);

        Self {
            normal,
            profundidade,
            pontos: buffer,
            quantidade: u8::try_from(pontos.len()).expect("no maximo MAX_PONTOS"),
        }
    }

    /// Direcao que aponta de A para B.
    #[must_use]
    pub fn normal(&self) -> Vec3 {
        self.normal
    }

    /// Quanto os corpos se interpenetram, sempre nao negativo.
    #[must_use]
    pub fn profundidade(&self) -> f32 {
        self.profundidade
    }

    /// Pontos na superficie de A, em ordem canonica.
    #[must_use]
    pub fn pontos(&self) -> &[Vec3] {
        &self.pontos[..usize::from(self.quantidade)]
    }

    /// Ponto correspondente na superficie de B.
    ///
    /// # Panics
    ///
    /// Se `i` estiver fora da faixa de [`pontos`](Self::pontos).
    #[must_use]
    pub fn ponto_em_b(&self, i: usize) -> Vec3 {
        self.pontos()[i] - self.normal * self.profundidade
    }

    /// O mesmo contato, visto de B para A.
    ///
    /// A normal inverte, a profundidade nao muda, e os pontos passam para a
    /// superficie do outro corpo. Aplicar duas vezes devolve o original.
    #[must_use]
    pub fn invertido(&self) -> Self {
        let n = usize::from(self.quantidade);
        let deslocamento = self.normal * self.profundidade;
        let mut buffer = [Vec3::ZERO; MAX_PONTOS];
        for (destino, origem) in buffer.iter_mut().zip(&self.pontos).take(n) {
            *destino = *origem - deslocamento;
        }
        Self::novo(-self.normal, self.profundidade, &buffer[..n])
    }
}

/// Ordem total sobre pontos: x, depois y, depois z.
///
/// `total_cmp` e nao `partial_cmp` porque um comparador que nao seja ordem total
/// faz `sort_by` entrar em panico, e `NaN` num ponto de contato e um bug que
/// deve aparecer como resultado errado, nao como travamento.
fn ordem_canonica(a: &Vec3, b: &Vec3) -> std::cmp::Ordering {
    a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)).then(a.z.total_cmp(&b.z))
}

// ----------------------------------------------------- grupo A: nucleo inflado --

/// Segmento engordado por um raio: a forma comum de esfera e capsula.
#[derive(Debug, Clone, Copy)]
struct Nucleo {
    inicio: Vec3,
    fim: Vec3,
    raio: f32,
}

impl Nucleo {
    fn esfera(raio: f32, pose: Pose) -> Self {
        Self { inicio: pose.posicao, fim: pose.posicao, raio }
    }

    fn capsula(raio: f32, meia_altura: f32, pose: Pose) -> Self {
        let eixo = pose.rotacao * (Vec3::Y * meia_altura);
        Self { inicio: pose.posicao - eixo, fim: pose.posicao + eixo, raio }
    }

    fn ponto(&self, s: f32) -> Vec3 {
        self.inicio + (self.fim - self.inicio) * s
    }

    fn comprimento(&self) -> f32 {
        (self.fim - self.inicio).length()
    }
}

/// Onde dois segmentos passam mais perto um do outro.
struct MaisProximos {
    /// Parametro em `[0,1]` sobre o primeiro segmento.
    s: f32,
    /// Parametro em `[0,1]` sobre o segundo.
    t: f32,
    /// Quando os segmentos sao paralelos e suas projecoes se sobrepoem, a faixa
    /// de `s` em que a distancia e a mesma. E dela que sai o manifold de dois
    /// pontos entre capsulas paralelas.
    intervalo: Option<(f32, f32)>,
}

/// Pontos mais proximos entre dois segmentos, por parametro.
fn mais_proximos(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> MaisProximos {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);

    let ponto1 = a <= EPS_COMPRIMENTO_QUADRADO;
    let ponto2 = e <= EPS_COMPRIMENTO_QUADRADO;

    // Esfera contra esfera: nao ha o que parametrizar.
    if ponto1 && ponto2 {
        return MaisProximos { s: 0.0, t: 0.0, intervalo: None };
    }
    if ponto1 {
        return MaisProximos { s: 0.0, t: (f / e).clamp(0.0, 1.0), intervalo: None };
    }

    let c = d1.dot(r);
    if ponto2 {
        return MaisProximos { s: (-c / a).clamp(0.0, 1.0), t: 0.0, intervalo: None };
    }

    let b = d1.dot(d2);
    let denominador = a * e - b * b;

    if denominador > EPS_PARALELO * a * e {
        let s = ((b * f - c * e) / denominador).clamp(0.0, 1.0);
        let t = (b * s + f) / e;

        // `t` fora do segmento significa que o ponto mais proximo esta numa das
        // pontas; ali `s` precisa ser recalculado contra essa ponta.
        if t < 0.0 {
            return MaisProximos { s: (-c / a).clamp(0.0, 1.0), t: 0.0, intervalo: None };
        }
        if t > 1.0 {
            return MaisProximos { s: ((b - c) / a).clamp(0.0, 1.0), t: 1.0, intervalo: None };
        }
        return MaisProximos { s, t, intervalo: None };
    }

    // Paralelos: a solucao do sistema e uma faixa, e nao um ponto. Projeta as
    // pontas do segundo segmento sobre o primeiro para achar a faixa.
    let s_de_p2 = -c / a;
    let s_de_q2 = (b - c) / a;
    let (bruto_lo, bruto_hi) =
        if s_de_p2 <= s_de_q2 { (s_de_p2, s_de_q2) } else { (s_de_q2, s_de_p2) };
    let lo = bruto_lo.max(0.0);
    let hi = bruto_hi.min(1.0);

    if lo <= hi {
        // O meio da faixa e simetrico: trocar A com B da o mesmo lugar. Escolher
        // uma das pontas nao daria.
        let s = (lo + hi) * 0.5;
        MaisProximos { s, t: ((b * s + f) / e).clamp(0.0, 1.0), intervalo: Some((lo, hi)) }
    } else {
        // Paralelos, mas as projecoes nao se cruzam: o contato e entre pontas.
        let s = if bruto_hi < 0.0 { 0.0 } else { 1.0 };
        MaisProximos { s, t: ((b * s + f) / e).clamp(0.0, 1.0), intervalo: None }
    }
}

/// Contato entre dois nucleos inflados.
fn contato_entre_nucleos(a: Nucleo, b: Nucleo) -> Option<Contato> {
    let proximos = mais_proximos(a.inicio, a.fim, b.inicio, b.fim);
    let pa = a.ponto(proximos.s);
    let pb = b.ponto(proximos.t);

    let delta = pb - pa;
    let distancia_quadrada = delta.length_squared();
    let soma_dos_raios = a.raio + b.raio;

    // Comparacao entre quadrados: a raiz nao mudaria a resposta.
    if distancia_quadrada > soma_dos_raios * soma_dos_raios {
        return None;
    }

    let (normal, distancia) = if distancia_quadrada > EPS_DIRECAO_QUADRADA {
        let d = distancia_quadrada.sqrt();
        (delta / d, d)
    } else {
        // Nucleos coincidentes: qualquer direcao serve, e nenhuma e melhor. `+Y`
        // e a escolha fixa, porque num mundo com gravidade em `−Y` separar para
        // cima e o menos surpreendente.
        (Vec3::Y, 0.0)
    };

    let profundidade = soma_dos_raios - distancia;

    let mut pontos = [Vec3::ZERO; MAX_PONTOS];
    let quantidade = match proximos.intervalo {
        Some((lo, hi)) if (hi - lo) * a.comprimento() > EPS_SEPARACAO_PONTOS => {
            pontos[0] = a.ponto(lo) + normal * a.raio;
            pontos[1] = a.ponto(hi) + normal * a.raio;
            2
        }
        _ => {
            pontos[0] = pa + normal * a.raio;
            1
        }
    };

    Some(Contato::novo(normal, profundidade, &pontos[..quantidade]))
}

/// Contato entre duas esferas.
#[must_use]
pub fn esfera_esfera(raio_a: f32, a: Pose, raio_b: f32, b: Pose) -> Option<Contato> {
    contato_entre_nucleos(Nucleo::esfera(raio_a, a), Nucleo::esfera(raio_b, b))
}

/// Contato entre uma esfera (A) e uma capsula (B).
#[must_use]
pub fn esfera_capsula(
    raio_a: f32,
    a: Pose,
    raio_b: f32,
    meia_altura_b: f32,
    b: Pose,
) -> Option<Contato> {
    contato_entre_nucleos(Nucleo::esfera(raio_a, a), Nucleo::capsula(raio_b, meia_altura_b, b))
}

/// Contato entre duas capsulas.
///
/// Capsulas paralelas que se sobrepoem produzem **dois** pontos, nas pontas da
/// faixa de sobreposicao. Um ponto so deixaria uma capsula deitada sobre outra
/// livre para girar.
#[must_use]
pub fn capsula_capsula(
    raio_a: f32,
    meia_altura_a: f32,
    a: Pose,
    raio_b: f32,
    meia_altura_b: f32,
    b: Pose,
) -> Option<Contato> {
    contato_entre_nucleos(
        Nucleo::capsula(raio_a, meia_altura_a, a),
        Nucleo::capsula(raio_b, meia_altura_b, b),
    )
}

// -------------------------------------------------------- grupo B: semiespaco --

/// Normal do plano no mundo, ja unitaria.
///
/// Devolve `None` quando a normal e degenerada — um plano sem orientacao valida
/// nao define semiespaco, e portanto nao define contato. E o unico caso em que a
/// forma em si esta fora do contrato, e ele e tratado em vez de ignorado.
fn normal_no_mundo(normal_local: Vec3, pose: Pose) -> Option<Vec3> {
    let n = pose.rotacao * normal_local;
    let comprimento_quadrado = n.length_squared();
    if comprimento_quadrado <= EPS_COMPRIMENTO_QUADRADO {
        return None;
    }
    Some(n / comprimento_quadrado.sqrt())
}

/// Distancia com sinal ao plano: positiva fora do solido.
fn altura_sobre_o_plano(ponto: Vec3, normal: Vec3, origem: Vec3) -> f32 {
    (ponto - origem).dot(normal)
}

/// Os oito vertices de uma caixa, em ordem fixa.
fn vertices_da_caixa(meias_extensoes: Vec3, pose: Pose) -> [Vec3; 8] {
    let m = Mat3::from_quat(pose.rotacao);
    let mut saida = [Vec3::ZERO; 8];
    let mut i = 0;

    for sinal_x in [-1.0_f32, 1.0] {
        for sinal_y in [-1.0_f32, 1.0] {
            for sinal_z in [-1.0_f32, 1.0] {
                let local = Vec3::new(
                    sinal_x * meias_extensoes.x,
                    sinal_y * meias_extensoes.y,
                    sinal_z * meias_extensoes.z,
                );
                saida[i] = pose.posicao + m * local;
                i += 1;
            }
        }
    }

    saida
}

/// Contato entre uma esfera (A) e um plano (B).
///
/// A normal do contato e a **oposta** a do plano: ela aponta de A para B, e o
/// solido do semiespaco fica do lado contrario a sua normal.
#[must_use]
pub fn esfera_plano(raio_a: f32, a: Pose, normal_b: Vec3, b: Pose) -> Option<Contato> {
    let n = normal_no_mundo(normal_b, b)?;
    let altura = altura_sobre_o_plano(a.posicao, n, b.posicao);

    if altura > raio_a {
        return None;
    }

    Some(Contato::novo(-n, raio_a - altura, &[a.posicao - n * raio_a]))
}

/// Contato entre uma capsula (A) e um plano (B).
///
/// Uma capsula deitada sobre o plano toca com as duas pontas e produz dois
/// pontos; em pe, um so.
#[must_use]
pub fn capsula_plano(
    raio_a: f32,
    meia_altura_a: f32,
    a: Pose,
    normal_b: Vec3,
    b: Pose,
) -> Option<Contato> {
    let n = normal_no_mundo(normal_b, b)?;
    let nucleo = Nucleo::capsula(raio_a, meia_altura_a, a);
    let extremos = [nucleo.inicio, nucleo.fim];

    let alturas = [
        altura_sobre_o_plano(extremos[0], n, b.posicao),
        altura_sobre_o_plano(extremos[1], n, b.posicao),
    ];
    let menor = alturas[0].min(alturas[1]);

    if menor > raio_a {
        return None;
    }

    let limite = menor + TOLERANCIA_COPLANAR;
    let mut pontos = [Vec3::ZERO; MAX_PONTOS];
    let mut quantidade = 0;

    for (extremo, altura) in extremos.iter().zip(alturas) {
        if altura > limite {
            continue;
        }
        let ponto = *extremo - n * raio_a;
        // Uma capsula de meia altura zero e uma esfera: as duas pontas caem no
        // mesmo lugar, e um ponto repetido nao e um manifold.
        if quantidade == 0 || pontos[0] != ponto {
            pontos[quantidade] = ponto;
            quantidade += 1;
        }
    }

    Some(Contato::novo(-n, raio_a - menor, &pontos[..quantidade]))
}

/// Contato entre uma caixa (A) e um plano (B).
///
/// Os pontos sao os vertices mais fundos. Uma face apoiada da quatro; uma aresta,
/// dois; um vertice, um. E [`TOLERANCIA_COPLANAR`] que separa "apoiada" de
/// "inclinada".
#[must_use]
pub fn caixa_plano(meias_extensoes_a: Vec3, a: Pose, normal_b: Vec3, b: Pose) -> Option<Contato> {
    let n = normal_no_mundo(normal_b, b)?;
    let vertices = vertices_da_caixa(meias_extensoes_a, a);

    let mut menor = f32::INFINITY;
    for vertice in vertices {
        menor = menor.min(altura_sobre_o_plano(vertice, n, b.posicao));
    }

    if menor > 0.0 {
        return None;
    }

    let limite = menor + TOLERANCIA_COPLANAR;
    let mut pontos = [Vec3::ZERO; MAX_PONTOS];
    let mut quantidade = 0;

    for vertice in vertices {
        if quantidade == MAX_PONTOS {
            break;
        }
        if altura_sobre_o_plano(vertice, n, b.posicao) > limite {
            continue;
        }
        // Uma caixa com alguma extensao zero tem vertices coincidentes.
        if pontos[..quantidade].contains(&vertice) {
            continue;
        }
        pontos[quantidade] = vertice;
        quantidade += 1;
    }

    Some(Contato::novo(-n, -menor, &pontos[..quantidade]))
}

// ------------------------------------------- grupo C: caixa vs nucleo inflado --

/// Contato entre uma esfera (A) e uma caixa (B).
///
/// Resolve em espaco local da caixa, onde a OBB vira uma AABB e o ponto mais
/// proximo e um `clamp`. Dois regimes:
///
/// - **centro fora da caixa** — o ponto mais proximo da superficie da a direcao
///   e a distancia, e o contato sai exato;
/// - **centro dentro da caixa** — nao ha direcao a extrair de uma distancia
///   zero. A saida e pela face de menor penetracao, que e a menor translacao que
///   resolve o contato.
///
/// O segundo regime nao e um caso raro a ignorar: e o que acontece quando uma
/// esfera pequena e empurrada para dentro de uma parede grossa, e sem ele a
/// normal seria indefinida justamente quando mais se precisa dela.
#[must_use]
pub fn esfera_caixa(raio_a: f32, a: Pose, meias_extensoes_b: Vec3, b: Pose) -> Option<Contato> {
    // A transposta de uma matriz de rotacao e a sua inversa, e nao precisa de
    // divisao — ao contrario de `Quat::inverse`.
    let m = Mat3::from_quat(b.rotacao);
    let local = m.transpose() * (a.posicao - b.posicao);
    let proximo = local.clamp(-meias_extensoes_b, meias_extensoes_b);

    let delta = proximo - local;
    let distancia_quadrada = delta.length_squared();

    if distancia_quadrada > EPS_DIRECAO_QUADRADA {
        if distancia_quadrada > raio_a * raio_a {
            return None;
        }

        let distancia = distancia_quadrada.sqrt();
        let normal = m * (delta / distancia);
        let ponto = a.posicao + normal * raio_a;

        return Some(Contato::novo(normal, raio_a - distancia, &[ponto]));
    }

    // Centro dentro (ou exatamente sobre a superficie). `folga` e quanto falta
    // para cada face em cada eixo; a menor delas e a saida mais curta.
    let folga = meias_extensoes_b - local.abs();
    let (eixo, saida) = if folga.x <= folga.y && folga.x <= folga.z {
        (Vec3::X, folga.x)
    } else if folga.y <= folga.z {
        (Vec3::Y, folga.y)
    } else {
        (Vec3::Z, folga.z)
    };

    // Empate entre eixos resolve em X, depois Y, depois Z — as comparacoes acima
    // usam `<=` para que o primeiro vença. Centro exatamente no plano medio de um
    // eixo nao define lado; ali a saida e no sentido positivo.
    let sentido = if eixo.dot(local) < 0.0 { -1.0 } else { 1.0 };

    // A normal aponta de A para B, e portanto **contra** a saida: separar move a
    // esfera por `−normal * profundidade`, que e para fora pela face escolhida.
    let normal = m * (eixo * -sentido);
    let ponto = a.posicao + normal * raio_a;

    Some(Contato::novo(normal, raio_a + saida, &[ponto]))
}

/// Quao perto do eixo a normal precisa estar para o contato ser de face.
///
/// Uma normal com componente dominante acima de `0,999` esta a menos de 2,6° de
/// um eixo da caixa. Acima disso o contato e de face, e a capsula deitada apoia
/// num trecho; abaixo, e de aresta ou de vertice, e um trecho seria geometria
/// inventada.
const EPS_EIXO_ALINHADO: f32 = 1e-3;

/// Parametro do ponto do segmento mais proximo da caixa, e a distancia.
///
/// A caixa esta alinhada e centrada na origem — quem chama transforma para o
/// espaco local dela antes.
///
/// # Como e exato sem iterar
///
/// A distancia ao quadrado ate uma caixa e `Σ max(0, |uᵢ| − hᵢ)²`. Ao longo do
/// segmento, cada termo e zero enquanto a coordenada esta dentro da fatia e um
/// quadrado de funcao linear quando esta fora. Os pontos onde isso muda sao
/// exatamente onde o segmento cruza um plano de face — no maximo seis. Entre
/// dois cruzamentos consecutivos o conjunto de termos ativos nao muda, entao a
/// funcao e uma **quadratica simples**, e o minimo dela sai por formula fechada.
///
/// Nao ha busca, nao ha iteracao, e nao ha tolerancia de convergencia: apenas
/// sete trechos no pior caso, cada um resolvido exatamente. Empate entre trechos
/// fica com o primeiro, que e o de menor `s`.
fn segmento_ate_caixa(p0: Vec3, p1: Vec3, meias: Vec3) -> (f32, f32) {
    let direcao = p1 - p0;
    let (p, d, h) = (p0.to_array(), direcao.to_array(), meias.to_array());

    // Os extremos sempre entram; os cruzamentos, quando caem dentro do segmento.
    let mut quebras = [0.0_f32; 8];
    let mut usadas = 2;
    quebras[1] = 1.0;

    for i in 0..3 {
        if d[i] * d[i] <= EPS_COMPRIMENTO_QUADRADO {
            continue;
        }
        for face in [h[i], -h[i]] {
            let s = (face - p[i]) / d[i];
            if s > 0.0 && s < 1.0 {
                quebras[usadas] = s;
                usadas += 1;
            }
        }
    }

    let quebras = &mut quebras[..usadas];
    quebras.sort_by(f32::total_cmp);

    let mut melhor = f32::INFINITY;
    let mut melhor_s = 0.0;

    for trecho in quebras.windows(2) {
        let (lo, hi) = (trecho[0], trecho[1]);
        if hi <= lo {
            continue;
        }

        // O conjunto de termos ativos e constante no trecho; o meio revela qual.
        let meio = (lo + hi) * 0.5;
        let (mut a, mut b, mut c) = (0.0_f32, 0.0_f32, 0.0_f32);

        for i in 0..3 {
            let u = p[i] + meio * d[i];
            let (constante, inclinacao) = if u > h[i] {
                (p[i] - h[i], d[i])
            } else if u < -h[i] {
                (-h[i] - p[i], -d[i])
            } else {
                (0.0, 0.0)
            };
            a += inclinacao * inclinacao;
            b += 2.0 * constante * inclinacao;
            c += constante * constante;
        }

        // `a` e soma de quadrados, entao a parabola abre para cima e o vertice e
        // o minimo. Com `a` zero a funcao e constante no trecho, e qualquer
        // ponto serve.
        let s = if a > 0.0 { (-b / (2.0 * a)).clamp(lo, hi) } else { lo };
        let valor = (a * s + b) * s + c;

        if valor < melhor {
            melhor = valor;
            melhor_s = s;
        }
    }

    (melhor_s, melhor.max(0.0))
}

/// Contato entre uma caixa (A) e uma capsula (B).
///
/// Dois regimes, e os dois sao exatos.
///
/// **Segmento fora da caixa.** A capsula e o segmento engordado por um raio,
/// entao a distancia entre segmento e caixa resolve tudo:
/// [`segmento_ate_caixa`] a calcula em forma fechada.
///
/// **Segmento atravessando a caixa.** Ali a distancia e zero e nao ha direcao a
/// extrair dela. A saida e o eixo separador de menor penetracao entre o segmento
/// e a caixa — os dois sao convexos, e o segmento e um poliedro degenerado, o
/// que da seis eixos candidatos: as tres faces da caixa e os tres produtos
/// vetoriais entre os eixos dela e a direcao do segmento.
///
/// O raio entra somando. Isso **nao** e uma aproximacao: crescer um convexo por
/// uma bola de raio `r` soma `r` a funcao suporte em toda direcao, entao o eixo
/// de menor penetracao e o mesmo e a profundidade cresce exatamente `r`. E por
/// isso que a capsula se resolve pelo segmento, e por isso que GJK e EPA nao
/// sao necessarios aqui.
#[must_use]
pub fn caixa_capsula(
    meias_extensoes_a: Vec3,
    a: Pose,
    raio_b: f32,
    meia_altura_b: f32,
    b: Pose,
) -> Option<Contato> {
    let m = Mat3::from_quat(a.rotacao);
    let para_local = m.transpose();
    let nucleo = Nucleo::capsula(raio_b, meia_altura_b, b);
    let p0 = para_local * (nucleo.inicio - a.posicao);
    let p1 = para_local * (nucleo.fim - a.posicao);
    let h = meias_extensoes_a;

    let (s, distancia_quadrada) = segmento_ate_caixa(p0, p1, h);

    if distancia_quadrada > raio_b * raio_b {
        return None;
    }

    if distancia_quadrada > EPS_DIRECAO_QUADRADA {
        return Some(contato_de_segmento_fora(p0, p1, h, s, distancia_quadrada, raio_b, a));
    }

    Some(contato_de_segmento_dentro(p0, p1, h, raio_b, a))
}

/// Segmento fora da caixa: o ponto mais proximo da a normal e a profundidade.
fn contato_de_segmento_fora(
    p0: Vec3,
    p1: Vec3,
    h: Vec3,
    s: f32,
    distancia_quadrada: f32,
    raio: f32,
    pose: Pose,
) -> Contato {
    let m = Mat3::from_quat(pose.rotacao);
    let direcao = p1 - p0;
    let no_segmento = p0 + direcao * s;
    let na_caixa = no_segmento.clamp(-h, h);

    let distancia = distancia_quadrada.sqrt();
    let normal_local = (no_segmento - na_caixa) / distancia;
    let profundidade = raio - distancia;

    // Contato de face: a normal e um eixo da caixa, e uma capsula deitada apoia
    // num trecho em vez de num ponto. O trecho sai recortando o segmento contra
    // as duas fatias perpendiculares a normal — as outras nao restringem, porque
    // a normal ja aponta para fora daquela face.
    let eixo = eixo_dominante(normal_local);
    let mut pontos = [Vec3::ZERO; MAX_PONTOS];
    let mut quantidade = 1;
    pontos[0] = pose.posicao + m * na_caixa;

    if let Some(indice) = eixo {
        if let Some((lo, hi)) = recorte_nas_outras_fatias(p0, direcao, h, indice) {
            let extremo_a = (p0 + direcao * lo).clamp(-h, h);
            let extremo_b = (p0 + direcao * hi).clamp(-h, h);
            if (extremo_b - extremo_a).length() > EPS_SEPARACAO_PONTOS {
                pontos[0] = pose.posicao + m * extremo_a;
                pontos[1] = pose.posicao + m * extremo_b;
                quantidade = 2;
            }
        }
    }

    Contato::novo(m * normal_local, profundidade, &pontos[..quantidade])
}

/// Segmento atravessando a caixa: eixo separador de menor penetracao.
fn contato_de_segmento_dentro(p0: Vec3, p1: Vec3, h: Vec3, raio: f32, pose: Pose) -> Contato {
    let m = Mat3::from_quat(pose.rotacao);
    let direcao = p1 - p0;
    let centro = (p0 + p1) * 0.5;
    let meio = direcao * 0.5;

    let candidatos = [
        Vec3::X,
        Vec3::Y,
        Vec3::Z,
        Vec3::X.cross(direcao),
        Vec3::Y.cross(direcao),
        Vec3::Z.cross(direcao),
    ];

    let mut melhor_separacao = f32::NEG_INFINITY;
    let mut melhor_eixo = Vec3::X;

    for candidato in candidatos {
        let comprimento_quadrado = candidato.length_squared();
        // Produto vetorial nulo: o segmento e paralelo aquele eixo da caixa, e o
        // eixo candidato nao existe. Descartar e correto, nao e um atalho.
        if comprimento_quadrado <= EPS_COMPRIMENTO_QUADRADO {
            continue;
        }

        let eixo = candidato / comprimento_quadrado.sqrt();
        let alcance = h.dot(eixo.abs()) + meio.dot(eixo).abs();
        let separacao = centro.dot(eixo).abs() - alcance;

        // Empate fica com o primeiro: a ordem dos candidatos e fixa.
        if separacao > melhor_separacao {
            melhor_separacao = separacao;
            melhor_eixo = if centro.dot(eixo) < 0.0 { -eixo } else { eixo };
        }
    }

    // A capsula e o segmento crescido pela bola de raio `r`, e crescer um convexo
    // por uma bola soma `r` a profundidade sem mudar a direcao.
    let profundidade = (raio - melhor_separacao).max(0.0);

    // Ponto de A mais fundo em B: o vertice de suporte na direcao da normal.
    // `signum` devolve `+1` para zero, o que fixa o desempate quando a normal e
    // perpendicular a um eixo.
    let suporte = Vec3::new(
        melhor_eixo.x.signum() * h.x,
        melhor_eixo.y.signum() * h.y,
        melhor_eixo.z.signum() * h.z,
    );

    Contato::novo(m * melhor_eixo, profundidade, &[pose.posicao + m * suporte])
}

/// Indice do eixo com que a normal esta alinhada, se houver.
fn eixo_dominante(normal: Vec3) -> Option<usize> {
    let a = normal.abs();
    let (indice, maior) = if a.x >= a.y && a.x >= a.z {
        (0, a.x)
    } else if a.y >= a.z {
        (1, a.y)
    } else {
        (2, a.z)
    };

    (1.0 - maior < EPS_EIXO_ALINHADO).then_some(indice)
}

/// Faixa de `s` em que o segmento fica dentro das fatias perpendiculares a
/// `excluido`.
///
/// `None` quando a faixa e vazia — o segmento passa ao largo da face.
fn recorte_nas_outras_fatias(
    p0: Vec3,
    direcao: Vec3,
    h: Vec3,
    excluido: usize,
) -> Option<(f32, f32)> {
    let (p, d, meias) = (p0.to_array(), direcao.to_array(), h.to_array());
    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);

    for i in 0..3 {
        if i == excluido {
            continue;
        }
        if d[i] * d[i] <= EPS_COMPRIMENTO_QUADRADO {
            // Sem variacao nesse eixo: ou o segmento inteiro esta na fatia, ou
            // nenhum ponto dele esta.
            if p[i].abs() > meias[i] {
                return None;
            }
            continue;
        }
        let t0 = (-meias[i] - p[i]) / d[i];
        let t1 = (meias[i] - p[i]) / d[i];
        let (menor, maior) = if t0 <= t1 { (t0, t1) } else { (t1, t0) };
        lo = lo.max(menor);
        hi = hi.min(maior);
    }

    (lo < hi).then_some((lo, hi))
}

/// Dois semiespacos nunca produzem contato finito.
///
/// Ou nao se cruzam, ou se cruzam num volume infinito — em nenhum dos casos ha
/// normal, profundidade ou ponto a devolver. A funcao existe para que a fronteira
/// seja explicita na matriz, e nao um caso esquecido.
#[must_use]
pub fn plano_plano(_normal_a: Vec3, _a: Pose, _normal_b: Vec3, _b: Pose) -> Option<Contato> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_1_SQRT_2;

    /// Um quarto de volta em torno de Z: leva o eixo Y da capsula para X.
    ///
    /// Construida por componentes, e nao por `Quat::from_rotation_z`, que
    /// chamaria seno e cosseno — as funcoes que a D09 mantem fora da simulacao.
    const QUARTO_Z: Quat = Quat::from_xyzw(0.0, 0.0, FRAC_1_SQRT_2, FRAC_1_SQRT_2);

    /// Meia volta em torno de X, exata em `f32`.
    const MEIA_VOLTA_X: Quat = Quat::from_xyzw(1.0, 0.0, 0.0, 0.0);

    /// Meia volta em torno de Y, exata em `f32`.
    const MEIA_VOLTA_Y: Quat = Quat::from_xyzw(0.0, 1.0, 0.0, 0.0);

    /// Um quarto de volta em torno de Y.
    const QUARTO_Y: Quat = Quat::from_xyzw(0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2);

    fn perto(a: Vec3, b: Vec3) -> bool {
        (a - b).abs().max_element() < 1e-5
    }

    /// Verifica as invariantes que a D15 impoe a qualquer contato.
    fn invariantes(c: &Contato) {
        assert!(c.profundidade() >= 0.0, "profundidade negativa: {}", c.profundidade());
        assert!(
            (c.normal().length() - 1.0).abs() < 1e-5,
            "normal nao unitaria: {}",
            c.normal().length()
        );
        assert!(!c.pontos().is_empty(), "contato sem ponto");
        assert!(c.pontos().len() <= MAX_PONTOS);
    }

    // ------------------------------------------------------- esfera x esfera --

    #[test]
    fn esferas_separadas_nao_tocam() {
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 2.001));

        assert_eq!(c, None);
    }

    #[test]
    fn esferas_tangentes_tocam_com_profundidade_zero() {
        // Encostar conta como contato, a mesma regra da broadphase.
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 2.0))
            .expect("tangencia e contato");
        invariantes(&c);

        assert_eq!(c.profundidade(), 0.0);
        assert_eq!(c.normal(), Vec3::X);
        assert_eq!(c.pontos(), [Vec3::X].as_slice());
        assert_eq!(c.ponto_em_b(0), Vec3::X, "sem penetracao, os pontos coincidem");
    }

    #[test]
    fn esferas_sobrepostas_dao_normal_profundidade_e_pontos() {
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::X, "de A para B");
        assert_eq!(c.profundidade(), 0.5);
        assert_eq!(c.pontos(), [Vec3::X].as_slice(), "ponto de A mais fundo em B");
        assert_eq!(c.ponto_em_b(0), Vec3::X * 0.5, "ponto de B mais fundo em A");
    }

    #[test]
    fn a_profundidade_bate_com_o_oraculo() {
        // Oraculo independente: soma dos raios menos a distancia entre centros.
        for (ca, cb, ra, rb) in [
            (Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 1.0_f32, 0.5_f32),
            (Vec3::new(-2.0, 1.0, 3.0), Vec3::new(-1.0, 1.5, 3.0), 2.0, 0.75),
            (Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, 5.5, 0.0), 3.0, 1.0),
        ] {
            let c = esfera_esfera(ra, Pose::em(ca), rb, Pose::em(cb)).expect("ha contato");
            let esperada = ra + rb - (cb - ca).length();

            assert!(
                (c.profundidade() - esperada).abs() < 1e-5,
                "profundidade {} != oraculo {esperada}",
                c.profundidade()
            );
        }
    }

    #[test]
    fn esferas_concentricas_usam_direcao_fixa() {
        // Nenhuma direcao e melhor que outra; a escolha precisa ser deterministica.
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 2.0, Pose::em(Vec3::ZERO))
            .expect("uma dentro da outra e contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::Y);
        assert_eq!(c.profundidade(), 3.0, "soma dos raios, distancia zero");
    }

    #[test]
    fn uma_esfera_dentro_da_outra() {
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 5.0, Pose::em(Vec3::X * 0.5))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::X);
        assert!((c.profundidade() - 5.5).abs() < 1e-6);
    }

    #[test]
    fn esferas_sao_simetricas() {
        let a = Pose::em(Vec3::new(1.0, 2.0, 3.0));
        let b = Pose::em(Vec3::new(1.5, 2.0, 3.0));

        let direto = esfera_esfera(1.0, a, 0.75, b).expect("ha contato");
        let inverso = esfera_esfera(0.75, b, 1.0, a).expect("ha contato").invertido();

        assert!(perto(direto.normal(), inverso.normal()));
        assert!((direto.profundidade() - inverso.profundidade()).abs() < 1e-6);
        assert!(perto(direto.pontos()[0], inverso.pontos()[0]));
    }

    #[test]
    fn inverter_duas_vezes_devolve_o_original() {
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5))
            .expect("ha contato");

        assert_eq!(c.invertido().invertido(), c);
    }

    #[test]
    fn inverter_troca_os_papeis_dos_pontos() {
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5))
            .expect("ha contato");
        let i = c.invertido();

        assert_eq!(i.normal(), -c.normal());
        assert_eq!(i.profundidade(), c.profundidade());
        assert_eq!(i.pontos()[0], c.ponto_em_b(0));
        assert_eq!(i.ponto_em_b(0), c.pontos()[0]);
    }

    // ------------------------------------------------------ esfera x capsula --

    #[test]
    fn esfera_longe_da_capsula_nao_toca() {
        let c = esfera_capsula(0.5, Pose::em(Vec3::X * 3.0), 0.5, 1.0, Pose::em(Vec3::ZERO));

        assert_eq!(c, None);
    }

    #[test]
    fn esfera_no_meio_do_flanco_da_capsula() {
        // Capsula em pe na origem, esfera ao lado na altura do meio.
        let c = esfera_capsula(0.5, Pose::em(Vec3::X * 0.8), 0.5, 1.0, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::NEG_X, "de A (esfera) para B (capsula)");
        assert!((c.profundidade() - 0.2).abs() < 1e-6);
        assert!(perto(c.pontos()[0], Vec3::new(0.3, 0.0, 0.0)));
    }

    #[test]
    fn esfera_acima_da_ponta_toca_a_calota() {
        // Acima do topo do segmento, a capsula e uma esfera centrada na ponta.
        let c = esfera_capsula(0.5, Pose::em(Vec3::Y * 1.8), 0.5, 1.0, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert!(perto(c.normal(), Vec3::NEG_Y));
        assert!((c.profundidade() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn capsula_de_meia_altura_zero_e_uma_esfera() {
        let como_capsula =
            esfera_capsula(1.0, Pose::em(Vec3::ZERO), 1.0, 0.0, Pose::em(Vec3::X * 1.5));
        let como_esfera = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5));

        assert_eq!(como_capsula, como_esfera);
    }

    #[test]
    fn esfera_no_eixo_da_capsula_usa_direcao_fixa() {
        let c = esfera_capsula(0.5, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::Y);
        assert_eq!(c.profundidade(), 1.0);
    }

    // ----------------------------------------------------- capsula x capsula --

    #[test]
    fn capsulas_paralelas_dao_dois_pontos() {
        // Duas capsulas em pe, lado a lado: o manifold precisa dos dois extremos,
        // senao uma poderia girar livremente sobre a outra.
        let c = capsula_capsula(1.0, 2.0, Pose::em(Vec3::ZERO), 1.0, 2.0, Pose::em(Vec3::X * 1.5))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2);
        assert_eq!(c.normal(), Vec3::X);
        assert!((c.profundidade() - 0.5).abs() < 1e-6);
        assert!(perto(c.pontos()[0], Vec3::new(1.0, -2.0, 0.0)));
        assert!(perto(c.pontos()[1], Vec3::new(1.0, 2.0, 0.0)));
    }

    #[test]
    fn capsulas_paralelas_sao_simetricas() {
        let a = Pose::em(Vec3::ZERO);
        let b = Pose::em(Vec3::X * 1.5);

        let direto = capsula_capsula(1.0, 2.0, a, 1.0, 2.0, b).expect("ha contato");
        let inverso = capsula_capsula(1.0, 2.0, b, 1.0, 2.0, a).expect("ha contato").invertido();

        assert!(perto(direto.normal(), inverso.normal()));
        assert!((direto.profundidade() - inverso.profundidade()).abs() < 1e-6);
        assert_eq!(direto.pontos().len(), inverso.pontos().len());
        for (p, q) in direto.pontos().iter().zip(inverso.pontos()) {
            assert!(perto(*p, *q), "{p} != {q}");
        }
    }

    #[test]
    fn capsulas_paralelas_com_sobreposicao_parcial() {
        // Deslocadas ao longo do eixo: a faixa comum encolhe, mas ainda existe.
        let c = capsula_capsula(
            1.0,
            2.0,
            Pose::em(Vec3::ZERO),
            1.0,
            2.0,
            Pose::em(Vec3::new(1.5, 3.0, 0.0)),
        )
        .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2);
        for p in c.pontos() {
            assert!(p.y >= 1.0 - 1e-5 && p.y <= 2.0 + 1e-5, "fora da faixa comum: {p}");
        }
    }

    #[test]
    fn capsulas_paralelas_sem_faixa_comum_dao_um_ponto() {
        // Uma acima da outra: as projecoes nao se cruzam, o contato e entre
        // pontas, e um ponto basta.
        let c = capsula_capsula(1.0, 2.0, Pose::em(Vec3::ZERO), 1.0, 2.0, Pose::em(Vec3::Y * 4.5))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 1);
        assert!(perto(c.normal(), Vec3::Y));
    }

    #[test]
    fn capsulas_cruzadas_dao_um_ponto() {
        // Uma em pe, outra deitada: os eixos nao sao paralelos.
        let c = capsula_capsula(
            0.5,
            2.0,
            Pose::em(Vec3::ZERO),
            0.5,
            2.0,
            Pose::nova(Vec3::X * 0.8, QUARTO_Z),
        )
        .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 1);
    }

    #[test]
    fn capsulas_separadas_nao_tocam() {
        let c = capsula_capsula(0.5, 1.0, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 5.0));

        assert_eq!(c, None);
    }

    #[test]
    fn capsula_invertida_no_eixo_e_a_mesma_capsula() {
        // Meia volta em X troca as pontas de lugar; a geometria nao muda.
        let normal =
            capsula_capsula(0.5, 1.0, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 0.8))
                .expect("ha contato");
        let girada = capsula_capsula(
            0.5,
            1.0,
            Pose::nova(Vec3::ZERO, MEIA_VOLTA_X),
            0.5,
            1.0,
            Pose::em(Vec3::X * 0.8),
        )
        .expect("ha contato");

        assert!(perto(normal.normal(), girada.normal()));
        assert!((normal.profundidade() - girada.profundidade()).abs() < 1e-6);
    }

    // -------------------------------------------------------- esfera x plano --

    #[test]
    fn esfera_acima_do_plano_nao_toca() {
        let c = esfera_plano(1.0, Pose::em(Vec3::Y * 1.5), Vec3::Y, Pose::em(Vec3::ZERO));

        assert_eq!(c, None);
    }

    #[test]
    fn esfera_tangente_ao_plano() {
        let c = esfera_plano(1.0, Pose::em(Vec3::Y), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("tangencia e contato");
        invariantes(&c);

        assert_eq!(c.profundidade(), 0.0);
        assert_eq!(c.normal(), Vec3::NEG_Y, "de A (esfera) para B (solido do plano)");
        assert_eq!(c.pontos(), [Vec3::ZERO].as_slice());
    }

    #[test]
    fn esfera_afundada_no_plano() {
        let c = esfera_plano(1.0, Pose::em(Vec3::Y * 0.75), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::NEG_Y);
        assert!((c.profundidade() - 0.25).abs() < 1e-6);
        assert_eq!(
            c.pontos(),
            [Vec3::new(0.0, -0.25, 0.0)].as_slice(),
            "o ponto mais baixo da esfera"
        );
        // O ponto correspondente em B e a projecao do centro sobre a superficie.
        assert_eq!(c.ponto_em_b(0), Vec3::ZERO);
    }

    #[test]
    fn esfera_com_centro_sobre_o_plano() {
        let c = esfera_plano(1.0, Pose::em(Vec3::ZERO), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.profundidade(), 1.0);
    }

    #[test]
    fn plano_sem_orientacao_valida_nao_produz_contato() {
        let c = esfera_plano(1.0, Pose::em(Vec3::ZERO), Vec3::ZERO, Pose::em(Vec3::ZERO));

        assert_eq!(c, None, "uma normal nula nao define semiespaco");
    }

    #[test]
    fn o_plano_acompanha_a_posicao_do_corpo() {
        // O plano passa pela origem local, entao mover o corpo move o plano.
        let alto = esfera_plano(1.0, Pose::em(Vec3::Y * 5.0), Vec3::Y, Pose::em(Vec3::Y * 4.5))
            .expect("ha contato");

        assert!((alto.profundidade() - 0.5).abs() < 1e-6);
    }

    // ------------------------------------------------------- capsula x plano --

    #[test]
    fn capsula_em_pe_toca_o_plano_com_um_ponto() {
        let c = capsula_plano(0.5, 1.0, Pose::em(Vec3::Y), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 1);
        assert_eq!(c.normal(), Vec3::NEG_Y);
        assert!((c.profundidade() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn capsula_deitada_toca_o_plano_com_dois_pontos() {
        let c = capsula_plano(
            0.5,
            1.0,
            Pose::nova(Vec3::Y * 0.4, QUARTO_Z),
            Vec3::Y,
            Pose::em(Vec3::ZERO),
        )
        .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2, "as duas pontas apoiam");
        assert!((c.profundidade() - 0.1).abs() < 1e-5);
        for p in c.pontos() {
            assert!((p.y + 0.1).abs() < 1e-5, "os dois pontos na mesma altura: {p}");
        }
    }

    #[test]
    fn capsula_acima_do_plano_nao_toca() {
        let c = capsula_plano(0.5, 1.0, Pose::em(Vec3::Y * 2.0), Vec3::Y, Pose::em(Vec3::ZERO));

        assert_eq!(c, None);
    }

    #[test]
    fn capsula_de_meia_altura_zero_no_plano_da_um_ponto_so() {
        let c = capsula_plano(0.5, 0.0, Pose::em(Vec3::Y * 0.25), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("ha contato");

        assert_eq!(c.pontos().len(), 1, "as duas pontas coincidem");
    }

    // --------------------------------------------------------- caixa x plano --

    #[test]
    fn caixa_apoiada_da_quatro_pontos() {
        // O caso que a D15 existe para suportar: uma face apoiada.
        let c = caixa_plano(Vec3::ONE, Pose::em(Vec3::Y * 0.5), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 4);
        assert!((c.profundidade() - 0.5).abs() < 1e-6);
        assert_eq!(c.normal(), Vec3::NEG_Y);
        for p in c.pontos() {
            assert!((p.y + 0.5).abs() < 1e-6, "os quatro cantos da face: {p}");
        }
    }

    #[test]
    fn caixa_tangente_ao_plano() {
        let c = caixa_plano(Vec3::ONE, Pose::em(Vec3::Y), Vec3::Y, Pose::em(Vec3::ZERO))
            .expect("encostar conta");
        invariantes(&c);

        assert_eq!(c.profundidade(), 0.0);
        assert_eq!(c.pontos().len(), 4);
    }

    #[test]
    fn caixa_acima_do_plano_nao_toca() {
        let c = caixa_plano(Vec3::ONE, Pose::em(Vec3::Y * 1.001), Vec3::Y, Pose::em(Vec3::ZERO));

        assert_eq!(c, None);
    }

    #[test]
    fn caixa_apoiada_numa_aresta_da_dois_pontos() {
        // Um quarto de volta em Z poe uma aresta para baixo? Nao: mantem uma
        // face. Meia volta tambem. Uma inclinacao qualquer poe um vertice.
        // A aresta aparece girando 45 graus em torno de Z.
        let meio_quarto_z = Quat::from_xyzw(0.0, 0.0, 0.382_683_43, 0.923_879_5);
        let c = caixa_plano(
            Vec3::ONE,
            Pose::nova(Vec3::Y * 1.2, meio_quarto_z),
            Vec3::Y,
            Pose::em(Vec3::ZERO),
        )
        .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2, "a aresta de baixo tem dois vertices");
        for p in c.pontos() {
            assert!((p.y - c.pontos()[0].y).abs() < 1e-5, "mesma altura");
        }
    }

    #[test]
    fn caixa_girada_um_quarto_continua_apoiando_a_face() {
        let quarto_y = Quat::from_xyzw(0.0, FRAC_1_SQRT_2, 0.0, FRAC_1_SQRT_2);
        let c = caixa_plano(
            Vec3::new(1.0, 2.0, 3.0),
            Pose::nova(Vec3::Y * 1.5, quarto_y),
            Vec3::Y,
            Pose::em(Vec3::ZERO),
        )
        .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 4, "girar em Y mantem a face para baixo");
        assert!((c.profundidade() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn caixa_com_extensao_zero_nao_repete_vertices() {
        // Uma caixa achatada tem vertices coincidentes; um manifold nao pode
        // conter o mesmo ponto duas vezes.
        let c = caixa_plano(
            Vec3::new(1.0, 0.0, 0.0),
            Pose::em(Vec3::ZERO),
            Vec3::Y,
            Pose::em(Vec3::ZERO),
        )
        .expect("ha contato");
        invariantes(&c);

        let pontos = c.pontos();
        for (i, p) in pontos.iter().enumerate() {
            for q in &pontos[i + 1..] {
                assert_ne!(p, q, "ponto repetido no manifold");
            }
        }
    }

    // --------------------------------------------------------- esfera x caixa --

    /// Distancia de um ponto a uma caixa alinhada centrada na origem.
    ///
    /// Formula fechada e independente da implementacao: serve de oraculo.
    fn distancia_ponto_caixa(ponto: Vec3, meias: Vec3) -> f32 {
        let fora = (ponto.abs() - meias).max(Vec3::ZERO);
        fora.length()
    }

    #[test]
    fn esfera_longe_da_caixa_nao_toca() {
        let c = esfera_caixa(0.5, Pose::em(Vec3::X * 3.0), Vec3::ONE, Pose::em(Vec3::ZERO));

        assert_eq!(c, None);
    }

    #[test]
    fn esfera_tangente_a_face_da_caixa() {
        let c = esfera_caixa(1.0, Pose::em(Vec3::X * 2.0), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("encostar conta");
        invariantes(&c);

        assert_eq!(c.profundidade(), 0.0);
        assert_eq!(c.normal(), Vec3::NEG_X, "de A (esfera) para B (caixa)");
        assert_eq!(c.pontos(), [Vec3::X].as_slice());
    }

    #[test]
    fn esfera_penetrando_a_face_da_caixa() {
        let c = esfera_caixa(1.0, Pose::em(Vec3::X * 1.5), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::NEG_X);
        assert!((c.profundidade() - 0.5).abs() < 1e-6);
        assert_eq!(c.pontos(), [Vec3::X * 0.5].as_slice());
        assert_eq!(c.ponto_em_b(0), Vec3::X, "o ponto da caixa mais proximo");
    }

    #[test]
    fn esfera_na_regiao_do_canto() {
        // Fora de todas as tres faces ao mesmo tempo: o ponto mais proximo e o
        // vertice, e a normal e diagonal.
        let centro = Vec3::splat(1.5);
        let c = esfera_caixa(1.0, Pose::em(centro), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        let esperada = (Vec3::ONE - centro).normalize();
        assert!(perto(c.normal(), esperada), "normal {} != {esperada}", c.normal());
        assert!(perto(c.ponto_em_b(0), Vec3::ONE), "o vertice da caixa");
    }

    #[test]
    fn esfera_na_regiao_da_aresta() {
        // Fora em dois eixos, dentro no terceiro.
        let centro = Vec3::new(1.4, 1.4, 0.0);
        let c = esfera_caixa(1.0, Pose::em(centro), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert!(perto(c.ponto_em_b(0), Vec3::new(1.0, 1.0, 0.0)), "um ponto da aresta");
        assert!(c.normal().z.abs() < 1e-6, "a normal nao sai do plano da aresta");
    }

    #[test]
    fn a_profundidade_bate_com_o_oraculo_de_ponto_caixa() {
        let meias = Vec3::new(1.0, 2.0, 0.5);
        for centro in [
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(1.2, 2.3, 0.0),
            Vec3::new(1.1, 2.1, 0.6),
            Vec3::new(0.0, 2.4, 0.0),
            Vec3::new(-1.3, -2.2, -0.7),
        ] {
            let raio = 1.0;
            let c = esfera_caixa(raio, Pose::em(centro), meias, Pose::em(Vec3::ZERO))
                .expect("ha contato");
            let esperada = raio - distancia_ponto_caixa(centro, meias);

            assert!(
                (c.profundidade() - esperada).abs() < 1e-5,
                "centro {centro}: {} != {esperada}",
                c.profundidade()
            );
        }
    }

    #[test]
    fn o_oraculo_concorda_sobre_a_ausencia_de_contato() {
        let meias = Vec3::new(1.0, 2.0, 0.5);
        for passo in 0..20u16 {
            let centro = Vec3::new(2.05 + f32::from(passo) * 0.1, 0.0, 0.0);
            let contato = esfera_caixa(1.0, Pose::em(centro), meias, Pose::em(Vec3::ZERO));

            assert!(distancia_ponto_caixa(centro, meias) > 1.0, "a cena precisa estar separada");
            assert_eq!(contato, None, "centro {centro}");
        }
    }

    #[test]
    fn esfera_dentro_da_caixa_sai_pela_face_mais_perto() {
        // Centro em (0,6, 0, 0) numa caixa unitaria: a face +X esta a 0,4, e as
        // outras a 1. A saida e por +X, entao a normal aponta para −X.
        let c = esfera_caixa(0.25, Pose::em(Vec3::X * 0.6), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("dentro tambem e contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::NEG_X);
        assert!((c.profundidade() - 0.65).abs() < 1e-6, "raio + folga = 0,25 + 0,4");
        // Separar move A por −normal * profundidade, e isso poe a esfera fora.
        let separada = Vec3::X * 0.6 - c.normal() * c.profundidade();
        assert!(separada.x >= 1.25 - 1e-5, "a esfera precisa sair da caixa: {separada}");
    }

    #[test]
    fn esfera_dentro_sai_pelo_lado_negativo_quando_e_mais_perto() {
        let c = esfera_caixa(0.25, Pose::em(Vec3::Y * -0.7), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::Y, "a saida e por −Y, entao a normal aponta para +Y");
    }

    #[test]
    fn esfera_no_centro_da_caixa_usa_o_menor_semi_eixo() {
        // Caixa achatada em Z: a saida mais curta e por Z, mesmo com o centro
        // exatamente no meio.
        let c =
            esfera_caixa(0.1, Pose::em(Vec3::ZERO), Vec3::new(3.0, 2.0, 0.5), Pose::em(Vec3::ZERO))
                .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.normal(), Vec3::NEG_Z, "sem lado definido, o sentido positivo");
        assert!((c.profundidade() - 0.6).abs() < 1e-6);
    }

    #[test]
    fn empate_entre_eixos_resolve_em_x() {
        // Cubo com a esfera no centro: os tres eixos empatam.
        let c = esfera_caixa(0.5, Pose::em(Vec3::ZERO), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");

        assert_eq!(c.normal(), Vec3::NEG_X);
    }

    #[test]
    fn a_caixa_girada_muda_qual_extensao_a_esfera_encontra() {
        // Meia volta em Y e exata em f32 e leva X em −X: uma caixa alongada em X
        // continua alongada em X.
        let meias = Vec3::new(2.0, 0.5, 0.5);
        let reta = esfera_caixa(1.0, Pose::em(Vec3::X * 2.5), meias, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        let girada =
            esfera_caixa(1.0, Pose::em(Vec3::X * 2.5), meias, Pose::nova(Vec3::ZERO, MEIA_VOLTA_Y))
                .expect("ha contato");

        assert_eq!(reta.profundidade(), girada.profundidade());
        assert_eq!(reta.normal(), girada.normal());
    }

    #[test]
    fn um_quarto_de_volta_troca_o_alcance_dos_eixos() {
        // Caixa alongada em X, girada um quarto em Y: passa a se estender em Z.
        let meias = Vec3::new(2.0, 0.5, 0.5);
        let pose = Pose::nova(Vec3::ZERO, QUARTO_Y);

        // Em X ela agora e curta: a 1,6 de distancia, com raio 1, ha contato
        // apenas porque a meia extensao virou 0,5.
        let em_x = esfera_caixa(1.0, Pose::em(Vec3::X * 1.4), meias, pose).expect("ha contato");
        assert!((em_x.profundidade() - 0.1).abs() < 1e-4, "{}", em_x.profundidade());

        // Em Z ela ficou longa, e a mesma distancia esta dentro dela.
        let em_z = esfera_caixa(1.0, Pose::em(Vec3::Z * 1.4), meias, pose).expect("ha contato");
        assert!(em_z.profundidade() > 1.0, "{}", em_z.profundidade());
    }

    #[test]
    fn a_caixa_acompanha_a_posicao() {
        let longe = Pose::em(Vec3::new(10.0, -5.0, 3.0));
        let c = esfera_caixa(1.0, Pose::em(Vec3::new(11.5, -5.0, 3.0)), Vec3::ONE, longe)
            .expect("ha contato");

        assert_eq!(c.normal(), Vec3::NEG_X);
        assert!((c.profundidade() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn inverter_o_contato_com_a_caixa_preserva_a_geometria() {
        let c = esfera_caixa(1.0, Pose::em(Vec3::X * 1.5), Vec3::ONE, Pose::em(Vec3::ZERO))
            .expect("ha contato");
        let i = c.invertido();

        assert_eq!(i.normal(), -c.normal());
        assert_eq!(i.profundidade(), c.profundidade());
        assert_eq!(i.pontos()[0], c.ponto_em_b(0));
        assert_eq!(i.invertido(), c);
    }

    #[test]
    fn esfera_caixa_e_deterministica() {
        let caso = || {
            (
                esfera_caixa(1.0, Pose::em(Vec3::X * 1.5), Vec3::ONE, Pose::em(Vec3::ZERO)),
                esfera_caixa(0.25, Pose::em(Vec3::X * 0.6), Vec3::ONE, Pose::em(Vec3::ZERO)),
                esfera_caixa(
                    1.0,
                    Pose::em(Vec3::splat(1.3)),
                    Vec3::ONE,
                    Pose::nova(Vec3::ZERO, QUARTO_Y),
                ),
            )
        };

        assert_eq!(caso(), caso());
    }

    // -------------------------------------------------------- caixa x capsula --

    /// Distancia ao quadrado de um ponto a uma caixa alinhada na origem.
    fn distancia_quadrada_ate_caixa(ponto: Vec3, meias: Vec3) -> f32 {
        (ponto.abs() - meias).max(Vec3::ZERO).length_squared()
    }

    /// Distancia entre segmento e caixa por amostragem densa.
    ///
    /// Oraculo independente: nao compartilha uma linha com a solucao fechada.
    /// Amostrar so pode **superestimar** o minimo, entao a analitica tem de ser
    /// menor ou igual, e a diferenca limitada pela resolucao da amostra.
    fn distancia_amostrada(p0: Vec3, p1: Vec3, meias: Vec3) -> f32 {
        let mut menor = f32::INFINITY;
        for i in 0..=4000u16 {
            let s = f32::from(i) / 4000.0;
            let ponto = p0 + (p1 - p0) * s;
            let fora = (ponto.abs() - meias).max(Vec3::ZERO);
            menor = menor.min(fora.length());
        }
        menor
    }

    #[test]
    fn a_distancia_fechada_concorda_com_a_amostrada() {
        let meias = Vec3::new(1.0, 2.0, 0.5);
        for (p0, p1) in [
            (Vec3::new(3.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0)),
            (Vec3::new(-4.0, 3.0, 2.0), Vec3::new(4.0, 3.0, 2.0)),
            (Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, 2.5, 0.0)),
            (Vec3::new(-3.0, -3.0, -3.0), Vec3::new(3.0, 3.0, 3.0)),
            (Vec3::new(2.0, 3.0, 1.0), Vec3::new(-2.0, 3.0, -1.0)),
            (Vec3::new(1.5, 0.0, 0.0), Vec3::new(1.5, 0.0, 0.0)),
        ] {
            let (_, quadrada) = segmento_ate_caixa(p0, p1, meias);
            let fechada = quadrada.sqrt();
            let amostrada = distancia_amostrada(p0, p1, meias);

            assert!(fechada <= amostrada + 1e-5, "fechada {fechada} > amostrada {amostrada}");
            assert!(amostrada - fechada < 5e-3, "fechada {fechada} longe de {amostrada}");
        }
    }

    #[test]
    fn o_parametro_devolvido_realiza_a_distancia() {
        let meias = Vec3::new(1.0, 2.0, 0.5);
        let (p0, p1) = (Vec3::new(-4.0, 3.0, 2.0), Vec3::new(4.0, 3.0, 2.0));
        let (s, quadrada) = segmento_ate_caixa(p0, p1, meias);

        let ponto = p0 + (p1 - p0) * s;
        let no_ponto = distancia_quadrada_ate_caixa(ponto, meias);

        assert!((0.0..=1.0).contains(&s), "s fora do segmento: {s}");
        assert!((no_ponto - quadrada).abs() < 1e-5, "{no_ponto} != {quadrada}");
    }

    #[test]
    fn capsula_longe_da_caixa_nao_toca() {
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 5.0));

        assert_eq!(c, None);
    }

    #[test]
    fn capsula_em_pe_tangente_a_face() {
        // Segmento vertical a 1,5 do centro; o raio 0,5 encosta na face x = 1.
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 1.5))
            .expect("encostar conta");
        invariantes(&c);

        assert_eq!(c.profundidade(), 0.0);
        assert!(perto(c.normal(), Vec3::X), "de A (caixa) para B (capsula)");
    }

    #[test]
    fn capsula_em_pe_penetrando_a_face() {
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 1.3))
            .expect("ha contato");
        invariantes(&c);

        assert!(perto(c.normal(), Vec3::X));
        // O segmento fica 0,3 fora da face, e o raio e 0,5.
        assert!((c.profundidade() - 0.2).abs() < 1e-5, "{}", c.profundidade());
    }

    #[test]
    fn capsula_deitada_sobre_a_face_apoia_em_dois_pontos() {
        // O caso que a matriz promete: o eixo paralelo a face da um trecho, e
        // nao um ponto. Um ponto so deixaria a capsula girar sobre a caixa.
        let deitada = Pose::nova(Vec3::Y * 1.4, QUARTO_Z);
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 2.0, deitada)
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2, "capsula deitada apoia num trecho");
        assert!(perto(c.normal(), Vec3::Y));
        assert!((c.profundidade() - 0.1).abs() < 1e-4, "{}", c.profundidade());
        for p in c.pontos() {
            assert!((p.y - 1.0).abs() < 1e-4, "os pontos ficam na face de cima: {p}");
            assert!(p.x.abs() <= 1.0 + 1e-4, "e dentro dela: {p}");
        }
    }

    #[test]
    fn capsula_deitada_curta_apoia_num_trecho_menor() {
        // Meia altura menor que a caixa: o trecho e o proprio segmento.
        let deitada = Pose::nova(Vec3::Y * 1.4, QUARTO_Z);
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 0.5, deitada)
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 2);
        let largura = (c.pontos()[1] - c.pontos()[0]).length();
        assert!((largura - 1.0).abs() < 1e-4, "o trecho e o segmento inteiro: {largura}");
    }

    #[test]
    fn capsula_alem_da_face_nao_apoia_em_dois_pontos() {
        // Deitada, mas passando da face: a normal deixa de ser um eixo da caixa
        // e o apoio vira um ponto na quina.
        let deitada = Pose::nova(Vec3::new(1.6, 1.4, 0.0), QUARTO_Z);
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 0.5, deitada)
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 1, "so a quina toca");
        assert!(perto(c.pontos()[0], Vec3::new(1.0, 1.0, 0.0)), "{}", c.pontos()[0]);
    }

    #[test]
    fn capsula_no_canto_da_caixa() {
        let canto = Pose::em(Vec3::new(1.3, 1.3, 1.3));
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 0.2, canto)
            .expect("ha contato");
        invariantes(&c);

        assert_eq!(c.pontos().len(), 1, "vertice nao apoia num trecho");
        assert!(perto(c.pontos()[0], Vec3::ONE), "o vertice da caixa: {}", c.pontos()[0]);
    }

    #[test]
    fn capsula_atravessando_a_caixa_usa_o_eixo_separador() {
        // O segmento passa pelo meio da caixa: nao ha distancia de onde tirar
        // direcao, e a saida e pelo eixo de menor penetracao.
        let atravessa = Pose::nova(Vec3::ZERO, QUARTO_Z);
        let c = caixa_capsula(Vec3::new(1.0, 2.0, 3.0), Pose::em(Vec3::ZERO), 0.25, 5.0, atravessa)
            .expect("atravessar e contato");
        invariantes(&c);

        // O segmento corre em X; a menor saida e pelo menor semi-eixo restante.
        assert!(c.profundidade() > 0.0);
        assert_eq!(c.pontos().len(), 1);
    }

    #[test]
    fn capsula_no_centro_da_caixa() {
        let c = caixa_capsula(
            Vec3::new(3.0, 2.0, 0.5),
            Pose::em(Vec3::ZERO),
            0.1,
            0.2,
            Pose::em(Vec3::ZERO),
        )
        .expect("ha contato");
        invariantes(&c);

        // O eixo de menor alcance e Z, e a capsula em pe soma meia altura em Y.
        assert!(perto(c.normal().abs(), Vec3::Z), "normal {}", c.normal());
        assert!((c.profundidade() - 0.6).abs() < 1e-5, "{}", c.profundidade());
    }

    #[test]
    fn a_profundidade_de_capsula_bate_com_a_distancia_amostrada() {
        let meias = Vec3::new(1.0, 2.0, 0.5);
        let raio = 0.75;
        for centro in [
            Vec3::new(1.5, 0.0, 0.0),
            Vec3::new(1.4, 2.3, 0.4),
            Vec3::new(0.0, 2.6, 0.0),
            Vec3::new(-1.6, -2.3, -0.8),
        ] {
            let pose = Pose::em(centro);
            let c = caixa_capsula(meias, Pose::em(Vec3::ZERO), raio, 0.4, pose)
                .expect("ha contato");

            let nucleo_inicio = centro - Vec3::Y * 0.4;
            let nucleo_fim = centro + Vec3::Y * 0.4;
            let esperada = raio - distancia_amostrada(nucleo_inicio, nucleo_fim, meias);

            assert!(
                (c.profundidade() - esperada).abs() < 5e-3,
                "centro {centro}: {} != {esperada}",
                c.profundidade()
            );
        }
    }

    #[test]
    fn a_caixa_girada_move_o_contato_com_a_capsula() {
        // Meia volta em Y e exata: uma caixa alongada em X continua alongada em X.
        let meias = Vec3::new(2.0, 0.5, 0.5);
        let capsula = Pose::em(Vec3::X * 2.4);

        let reta = caixa_capsula(meias, Pose::em(Vec3::ZERO), 0.5, 0.3, capsula)
            .expect("ha contato");
        let girada = caixa_capsula(meias, Pose::nova(Vec3::ZERO, MEIA_VOLTA_Y), 0.5, 0.3, capsula)
            .expect("ha contato");

        assert!((reta.profundidade() - girada.profundidade()).abs() < 1e-6);
    }

    #[test]
    fn inverter_o_contato_com_a_capsula_preserva_a_geometria() {
        let c = caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 1.3))
            .expect("ha contato");
        let i = c.invertido();

        assert_eq!(i.normal(), -c.normal());
        assert_eq!(i.profundidade(), c.profundidade());
        assert_eq!(i.invertido(), c);
    }

    #[test]
    fn caixa_capsula_e_deterministica() {
        let caso = || {
            (
                caixa_capsula(Vec3::ONE, Pose::em(Vec3::ZERO), 0.5, 1.0, Pose::em(Vec3::X * 1.3)),
                caixa_capsula(
                    Vec3::ONE,
                    Pose::em(Vec3::ZERO),
                    0.5,
                    2.0,
                    Pose::nova(Vec3::Y * 1.4, QUARTO_Z),
                ),
                caixa_capsula(
                    Vec3::new(1.0, 2.0, 3.0),
                    Pose::em(Vec3::ZERO),
                    0.25,
                    5.0,
                    Pose::nova(Vec3::ZERO, QUARTO_Z),
                ),
            )
        };

        assert_eq!(caso(), caso());
    }

    #[test]
    fn o_recorte_recusa_faixa_vazia() {
        // Segmento paralelo a X, mas fora da fatia em Z: nao ha apoio de face.
        let vazio =
            recorte_nas_outras_fatias(Vec3::new(-3.0, 0.0, 5.0), Vec3::X * 6.0, Vec3::ONE, 1);

        assert_eq!(vazio, None);
    }

    // --------------------------------------------------------- plano x plano --

    #[test]
    fn dois_planos_nunca_produzem_contato() {
        for (na, nb) in
            [(Vec3::Y, Vec3::Y), (Vec3::Y, Vec3::NEG_Y), (Vec3::Y, Vec3::X), (Vec3::ZERO, Vec3::Y)]
        {
            assert_eq!(plano_plano(na, Pose::em(Vec3::ZERO), nb, Pose::em(Vec3::X)), None);
        }
    }

    // ------------------------------------------------------------- manifold --

    #[test]
    fn os_pontos_saem_em_ordem_canonica() {
        let c = Contato::novo(
            Vec3::Y,
            1.0,
            &[
                Vec3::new(2.0, 0.0, 0.0),
                Vec3::new(1.0, 5.0, 0.0),
                Vec3::new(1.0, 0.0, 9.0),
                Vec3::new(1.0, 0.0, 3.0),
            ],
        );

        let esperado = [
            Vec3::new(1.0, 0.0, 3.0),
            Vec3::new(1.0, 0.0, 9.0),
            Vec3::new(1.0, 5.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
        ];
        assert_eq!(c.pontos(), esperado.as_slice());
    }

    #[test]
    fn a_ordem_de_entrada_nao_muda_o_contato() {
        let pontos = [Vec3::new(2.0, 0.0, 0.0), Vec3::new(1.0, 5.0, 0.0), Vec3::new(1.0, 0.0, 9.0)];
        let mut ao_contrario = pontos;
        ao_contrario.reverse();

        assert_eq!(
            Contato::novo(Vec3::Y, 1.0, &pontos),
            Contato::novo(Vec3::Y, 1.0, &ao_contrario)
        );
    }

    #[test]
    #[should_panic(expected = "ao menos um ponto")]
    fn contato_sem_ponto_e_recusado() {
        let _ = Contato::novo(Vec3::Y, 1.0, &[]);
    }

    #[test]
    #[should_panic(expected = "no maximo")]
    fn manifold_acima_do_limite_e_recusado() {
        let _ = Contato::novo(Vec3::Y, 1.0, &[Vec3::ZERO; MAX_PONTOS + 1]);
    }

    // ---------------------------------------------------------- determinismo --

    #[test]
    fn a_mesma_cena_produz_o_mesmo_contato_bit_a_bit() {
        let casos = || {
            (
                esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5)),
                capsula_capsula(
                    1.0,
                    2.0,
                    Pose::em(Vec3::ZERO),
                    1.0,
                    2.0,
                    Pose::nova(Vec3::X * 1.5, QUARTO_Z),
                ),
                caixa_plano(Vec3::ONE, Pose::em(Vec3::Y * 0.5), Vec3::Y, Pose::em(Vec3::ZERO)),
                capsula_plano(0.5, 1.0, Pose::em(Vec3::Y * 0.9), Vec3::Y, Pose::em(Vec3::ZERO)),
            )
        };

        assert_eq!(casos(), casos());
    }

    #[test]
    fn o_contato_de_esferas_e_bit_a_bit_o_esperado() {
        // Trava o resultado exato: uma reformulacao que passe a usar
        // trigonometria muda estes bits.
        let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * 1.5))
            .expect("ha contato");

        assert_eq!(c.profundidade().to_bits(), 0.5_f32.to_bits());
        assert_eq!(c.normal().x.to_bits(), 1.0_f32.to_bits());
        assert_eq!(c.pontos()[0].x.to_bits(), 1.0_f32.to_bits());
    }

    // ------------------------------------------- propriedades, e nao valores --

    #[test]
    fn nunca_ha_contato_quando_os_corpos_estao_separados() {
        // Varre distancias acima da soma dos raios: nenhuma pode gerar contato.
        for passo in 1..40u16 {
            let d = 2.0 + f32::from(passo) * 0.05;
            assert_eq!(
                esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * d)),
                None,
                "distancia {d}"
            );
        }
    }

    #[test]
    fn o_ponto_em_b_fica_sempre_na_superficie_de_b() {
        // Propriedade geometrica: para esfera contra esfera, o ponto em B esta a
        // exatamente um raio do centro de B.
        // Ate 1,64, abaixo da soma dos raios (1,75): acima disso nao ha contato.
        for passo in 0..25u16 {
            let d = 0.2 + f32::from(passo) * 0.06;
            let centro_b = Vec3::new(d, 0.0, 0.0);
            let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 0.75, Pose::em(centro_b))
                .expect("ha contato");

            let distancia = (c.ponto_em_b(0) - centro_b).length();
            assert!((distancia - 0.75).abs() < 1e-4, "d={d}, distancia {distancia}");
        }
    }

    #[test]
    fn a_profundidade_cresce_conforme_os_corpos_se_aproximam() {
        let mut anterior = f32::NEG_INFINITY;
        for passo in 0..30u16 {
            let d = 1.9 - f32::from(passo) * 0.06;
            let c = esfera_esfera(1.0, Pose::em(Vec3::ZERO), 1.0, Pose::em(Vec3::X * d))
                .expect("ha contato");

            assert!(c.profundidade() >= anterior - 1e-6, "profundidade nao monotona em d={d}");
            anterior = c.profundidade();
        }
    }
}
