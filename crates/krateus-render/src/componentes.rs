//! O que a simulacao precisa declarar para aparecer na tela.
//!
//! Estes componentes vivem no mundo logico, mas sao definidos aqui porque quem
//! define o contrato de extracao e quem extrai. A simulacao nao precisa saber
//! nada sobre renderizacao alem de acrescenta-los.

use glam::{Mat4, Quat, Vec3};
use krateus_ecs::Entity;

/// Identificador de malha.
///
/// Opaco de proposito: a pipeline de assets da Fase 6 e quem vai resolve-lo
/// para dados de verdade. Ate la, e so uma chave de agrupamento.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MeshId(pub u32);

/// Identificador de material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MaterialId(pub u32);

/// Posicao, rotacao e escala de uma entidade.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    /// Deslocamento em espaco de mundo.
    pub translacao: Vec3,
    /// Orientacao.
    pub rotacao: Quat,
    /// Escala por eixo.
    pub escala: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { translacao: Vec3::ZERO, rotacao: Quat::IDENTITY, escala: Vec3::ONE }
    }
}

impl Transform {
    /// Transform sem rotacao nem escala.
    #[must_use]
    pub fn em(x: f32, y: f32, z: f32) -> Self {
        Self { translacao: Vec3::new(x, y, z), ..Self::default() }
    }

    /// Matriz de modelo correspondente.
    #[must_use]
    pub fn matriz(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.escala, self.rotacao, self.translacao)
    }

    /// Maior fator de escala entre os tres eixos.
    ///
    /// E o que multiplica o raio da esfera envolvente: uma esfera escalada de
    /// forma nao uniforme vira elipsoide, e usar o maior eixo mantem o teste de
    /// visibilidade conservador — pode aceitar algo invisivel, nunca descartar
    /// algo visivel.
    #[must_use]
    pub fn escala_maxima(&self) -> f32 {
        self.escala.x.abs().max(self.escala.y.abs()).max(self.escala.z.abs())
    }
}

/// Declara que a entidade deve ser desenhada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Renderable {
    /// Malha a desenhar.
    pub mesh: MeshId,
    /// Material com que desenhar.
    pub material: MaterialId,
}

/// Esfera envolvente em espaco local.
///
/// Esfera, e nao caixa, porque o teste contra o frustum e uma comparacao de
/// distancia por plano — seis operacoes, sem depender da rotacao. Uma caixa
/// alinhada aos eixos precisaria ser recalculada a cada rotacao.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Raio em espaco local.
    pub raio: f32,
}

impl Default for Bounds {
    fn default() -> Self {
        Self { raio: 1.0 }
    }
}

/// Permite ocultar uma entidade sem remove-la nem tirar seus componentes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Visibility {
    /// Se falso, a entidade nao entra no snapshot.
    pub visivel: bool,
}

impl Default for Visibility {
    fn default() -> Self {
        Self { visivel: true }
    }
}

impl Visibility {
    /// Visivel.
    pub const VISIVEL: Self = Self { visivel: true };
    /// Oculta.
    pub const OCULTA: Self = Self { visivel: false };
}

/// Camera em perspectiva.
///
/// A entidade que a carrega precisa ter tambem um [`Transform`], que da a
/// posicao e a orientacao.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// Campo de visao vertical, em radianos.
    pub fov_y: f32,
    /// Plano de corte proximo.
    pub perto: f32,
    /// Plano de corte distante.
    pub longe: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { fov_y: std::f32::consts::FRAC_PI_4, perto: 0.1, longe: 1000.0 }
    }
}

/// Recurso que aponta qual entidade e a camera do quadro.
///
/// Um recurso, e nao um componente-marcador, porque a pergunta "qual e a camera
/// ativa" tem exatamente uma resposta por mundo, e essa e a diferenca entre
/// recurso e componente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CameraAtiva(pub Option<Entity>);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transform_padrao_e_identidade() {
        assert_eq!(Transform::default().matriz(), Mat4::IDENTITY);
    }

    #[test]
    fn matriz_aplica_translacao() {
        let t = Transform::em(1.0, 2.0, 3.0);
        let p = t.matriz().transform_point3(Vec3::ZERO);
        assert_eq!(p, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn escala_maxima_pega_o_maior_eixo() {
        let t = Transform { escala: Vec3::new(1.0, 5.0, 2.0), ..Transform::default() };
        assert_eq!(t.escala_maxima(), 5.0);
    }

    #[test]
    fn escala_maxima_ignora_o_sinal() {
        // Escala negativa espelha; o tamanho envolvente e o mesmo.
        let t = Transform { escala: Vec3::new(-7.0, 1.0, 1.0), ..Transform::default() };
        assert_eq!(t.escala_maxima(), 7.0);
    }

    #[test]
    fn visibilidade_padrao_e_visivel() {
        assert_eq!(Visibility::default(), Visibility::VISIVEL);
    }
}
