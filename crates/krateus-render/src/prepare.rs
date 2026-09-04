//! Estagio 2: do snapshot para dados prontos para a GPU.
//!
//! Aqui entra o que depende da tela — proporcao, matriz de projecao — e o que
//! depende da camera: descartar o que nao aparece.
//!
//! O culling vive neste estagio, e nao no [`extract`](crate::extract), por um
//! motivo pratico: a extracao acontece uma vez por passo de simulacao, e o
//! preparo pode acontecer uma vez por *vista* — uma camera de jogo, um mapa,
//! uma sombra. Descartar na extracao obrigaria a extrair de novo para cada
//! vista.

use glam::{Mat4, Vec3, Vec4};

use crate::extract::{InstanceSnapshot, RenderSnapshot};

/// Volume de visao, como seis planos.
///
/// Os planos saem direto da matriz combinada, pelo metodo de Gribb e Hartmann:
/// cada plano e uma soma ou diferenca de duas linhas da matriz. Nao ha
/// trigonometria envolvida, o que importa para o determinismo (D09) — funcoes
/// transcendentais sao justamente onde plataformas divergem.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frustum {
    /// Esquerda, direita, baixo, cima, perto, longe. Cada plano e `(nx, ny, nz, d)`
    /// com a normal apontando para dentro.
    planos: [Vec4; 6],
}

impl Frustum {
    /// Extrai os planos de uma matriz de visao-projecao.
    #[must_use]
    pub fn de_view_proj(m: Mat4) -> Self {
        // `glam` guarda por coluna; as linhas sao o que o metodo pede.
        let l0 = Vec4::new(m.x_axis.x, m.y_axis.x, m.z_axis.x, m.w_axis.x);
        let l1 = Vec4::new(m.x_axis.y, m.y_axis.y, m.z_axis.y, m.w_axis.y);
        let l2 = Vec4::new(m.x_axis.z, m.y_axis.z, m.z_axis.z, m.w_axis.z);
        let l3 = Vec4::new(m.x_axis.w, m.y_axis.w, m.z_axis.w, m.w_axis.w);

        let planos = [l3 + l0, l3 - l0, l3 + l1, l3 - l1, l3 + l2, l3 - l2];
        Self { planos: planos.map(normaliza) }
    }

    /// Indica se a esfera cruza ou esta dentro do volume.
    ///
    /// Conservador: pode aceitar algo que nao aparece, nunca descarta algo que
    /// aparece. Um culling que erra para o outro lado produz buraco na tela, e
    /// buraco e muito mais caro de diagnosticar do que um desenho a mais.
    #[must_use]
    pub fn contem_esfera(&self, centro: Vec3, raio: f32) -> bool {
        self.planos.iter().all(|p| distancia(*p, centro) >= -raio)
    }
}

fn normaliza(p: Vec4) -> Vec4 {
    let tamanho = Vec3::new(p.x, p.y, p.z).length();
    if tamanho > 0.0 { p / tamanho } else { p }
}

fn distancia(plano: Vec4, ponto: Vec3) -> f32 {
    plano.x * ponto.x + plano.y * ponto.y + plano.z * ponto.z + plano.w
}

/// O quadro depois do preparo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PreparedFrame {
    /// Visao combinada com projecao.
    pub view_proj: Mat4,
    /// O que sobrou depois do culling, na ordem do snapshot.
    pub visiveis: Vec<InstanceSnapshot>,
    /// Quantas instancias o culling descartou. Alimenta o profiler da Fase 7.
    pub descartadas: usize,
}

/// Prepara o snapshot para uma superficie de proporcao `aspecto`.
///
/// Sem camera nao ha o que preparar: devolve um quadro vazio em vez de escolher
/// uma camera arbitraria, que esconderia o problema.
#[must_use]
pub fn prepare(snapshot: &RenderSnapshot, aspecto: f32) -> PreparedFrame {
    let Some(camera) = snapshot.camera else {
        return PreparedFrame::default();
    };

    let view_proj = camera.projecao(aspecto) * camera.view;
    let frustum = Frustum::de_view_proj(view_proj);

    let mut visiveis = Vec::with_capacity(snapshot.instancias.len());
    for i in &snapshot.instancias {
        if frustum.contem_esfera(i.centro, i.raio) {
            visiveis.push(*i);
        }
    }

    let descartadas = snapshot.instancias.len() - visiveis.len();
    PreparedFrame { view_proj, visiveis, descartadas }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::componentes::{MaterialId, MeshId};
    use crate::extract::CameraSnapshot;
    use krateus_ecs::Entity;
    use std::num::NonZeroU32;

    fn camera_na_origem_olhando_para_menos_z() -> CameraSnapshot {
        CameraSnapshot {
            view: Mat4::IDENTITY,
            fov_y: std::f32::consts::FRAC_PI_2,
            perto: 0.1,
            longe: 100.0,
        }
    }

    fn instancia(n: u32, centro: Vec3, raio: f32) -> InstanceSnapshot {
        InstanceSnapshot {
            entity: Entity::from_raw(n, NonZeroU32::new(1).expect("nao nulo")),
            modelo: Mat4::from_translation(centro),
            mesh: MeshId(0),
            material: MaterialId(0),
            raio,
            centro,
        }
    }

    fn snapshot(instancias: Vec<InstanceSnapshot>) -> RenderSnapshot {
        RenderSnapshot { camera: Some(camera_na_origem_olhando_para_menos_z()), instancias }
    }

    #[test]
    fn sem_camera_nada_e_preparado() {
        let s = RenderSnapshot { camera: None, instancias: vec![instancia(0, Vec3::ZERO, 1.0)] };
        let f = prepare(&s, 16.0 / 9.0);

        assert!(f.visiveis.is_empty());
        assert_eq!(f.descartadas, 0, "sem camera nao ha culling, ha ausencia de quadro");
    }

    #[test]
    fn objeto_a_frente_e_visivel() {
        let s = snapshot(vec![instancia(0, Vec3::new(0.0, 0.0, -10.0), 1.0)]);
        let f = prepare(&s, 1.0);

        assert_eq!(f.visiveis.len(), 1);
        assert_eq!(f.descartadas, 0);
    }

    #[test]
    fn objeto_atras_da_camera_e_descartado() {
        let s = snapshot(vec![instancia(0, Vec3::new(0.0, 0.0, 10.0), 1.0)]);
        let f = prepare(&s, 1.0);

        assert!(f.visiveis.is_empty());
        assert_eq!(f.descartadas, 1);
    }

    #[test]
    fn objeto_alem_do_plano_distante_e_descartado() {
        let s = snapshot(vec![instancia(0, Vec3::new(0.0, 0.0, -500.0), 1.0)]);
        assert_eq!(prepare(&s, 1.0).descartadas, 1);
    }

    #[test]
    fn objeto_muito_a_esquerda_e_descartado() {
        let s = snapshot(vec![instancia(0, Vec3::new(-1000.0, 0.0, -10.0), 1.0)]);
        assert_eq!(prepare(&s, 1.0).descartadas, 1);
    }

    #[test]
    fn raio_grande_salva_objeto_que_o_centro_perderia() {
        // O centro esta fora, mas a esfera cruza o volume. Descartar aqui
        // produziria um buraco na tela.
        let centro = Vec3::new(0.0, 0.0, 12.0);
        let s = snapshot(vec![instancia(0, centro, 20.0)]);

        assert_eq!(prepare(&s, 1.0).visiveis.len(), 1);
    }

    #[test]
    fn culling_preserva_a_ordem_do_snapshot() {
        let s = snapshot(vec![
            instancia(0, Vec3::new(0.0, 0.0, -5.0), 1.0),
            instancia(1, Vec3::new(0.0, 0.0, 500.0), 1.0), // atras, descartado
            instancia(2, Vec3::new(0.0, 0.0, -6.0), 1.0),
        ]);

        let f = prepare(&s, 1.0);
        let ids: Vec<u32> = f.visiveis.iter().map(|i| i.entity.index()).collect();

        assert_eq!(ids, vec![0, 2]);
        assert_eq!(f.descartadas, 1);
    }

    #[test]
    fn preparo_repetido_produz_o_mesmo_quadro() {
        let s =
            snapshot((0..30).map(|i| instancia(i, Vec3::new(i as f32, 0.0, -20.0), 1.0)).collect());

        assert_eq!(prepare(&s, 1.6), prepare(&s, 1.6));
    }

    #[test]
    fn proporcao_diferente_muda_o_que_aparece() {
        // Uma tela mais larga enxerga mais para os lados.
        let s = snapshot(vec![instancia(0, Vec3::new(14.0, 0.0, -10.0), 0.5)]);

        assert_eq!(prepare(&s, 1.0).visiveis.len(), 0);
        assert_eq!(prepare(&s, 4.0).visiveis.len(), 1);
    }

    #[test]
    fn planos_do_frustum_ficam_normalizados() {
        // Sem normalizar, a distancia deixa de ser distancia e o teste de esfera
        // passa a comparar raio com um numero em outra escala.
        let vp = Mat4::perspective_rh(1.0, 1.5, 0.1, 100.0);
        let f = Frustum::de_view_proj(vp);

        for p in f.planos {
            let n = Vec3::new(p.x, p.y, p.z).length();
            assert!((n - 1.0).abs() < 1e-4, "plano com normal de tamanho {n}");
        }
    }
}
