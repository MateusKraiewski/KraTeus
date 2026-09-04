//! Estagio 1: transformar o mundo logico num snapshot de render.
//!
//! Este e o ponto onde a secao 10 do documento de visao se realiza. Antes dele,
//! existe simulacao; depois, existe um retrato **fechado** dela.
//!
//! # O snapshot nao referencia o mundo
//!
//! [`RenderSnapshot`] e feito de valores proprios — sem `&World`, sem tempo de
//! vida, sem identificador que precise ser resolvido depois. Isso nao e detalhe
//! de estilo:
//!
//! - A simulacao pode avancar, criar e destruir entidades enquanto o quadro
//!   anterior ainda esta sendo desenhado.
//! - O snapshot pode cruzar threads sem levar o mundo junto.
//! - Uma entidade destruida logo apos a extracao nao deixa referencia pendurada;
//!   o retrato dela ja e uma copia.
//!
//! # Determinismo
//!
//! A extracao percorre o ECS na ordem estavel dele — archetypes na ordem de
//! criacao, linhas em ordem crescente. A mesma sequencia de operacoes produz o
//! mesmo snapshot.
//!
//! Isso e mais fraco que "mesmo estado logico produz o mesmo snapshot": dois
//! mundos com o mesmo conteudo mas historicos diferentes podem extrair em ordem
//! diferente. A forma canonica aparece no estagio de
//! [`queue`](crate::queue), que ordena por material e malha. Ordenar aqui seria
//! trabalho jogado fora.

use glam::Mat4;
use krateus_ecs::{Entity, World};

use crate::componentes::{
    Bounds, Camera, CameraAtiva, MaterialId, MeshId, Renderable, Transform, Visibility,
};

/// Uma entidade desenhavel, ja desacoplada do mundo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InstanceSnapshot {
    /// De qual entidade este retrato veio. Serve a diagnostico e a selecao por
    /// clique; o renderer nao a usa para nada.
    pub entity: Entity,
    /// Matriz de modelo ja calculada.
    pub modelo: Mat4,
    /// Malha.
    pub mesh: MeshId,
    /// Material.
    pub material: MaterialId,
    /// Raio da esfera envolvente **em espaco de mundo**, ja escalado.
    pub raio: f32,
    /// Centro da esfera envolvente em espaco de mundo.
    pub centro: glam::Vec3,
}

/// A camera do quadro, ja resolvida.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraSnapshot {
    /// Matriz de visao.
    pub view: Mat4,
    /// Campo de visao vertical, em radianos.
    pub fov_y: f32,
    /// Plano proximo.
    pub perto: f32,
    /// Plano distante.
    pub longe: f32,
}

impl CameraSnapshot {
    /// Matriz de projecao para uma dada proporcao de tela.
    ///
    /// A projecao fica de fora do snapshot porque depende do tamanho da
    /// superficie, que pertence ao renderer e nao a simulacao. Redimensionar a
    /// janela nao deve invalidar um snapshot ja extraido.
    #[must_use]
    pub fn projecao(&self, aspecto: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, aspecto, self.perto, self.longe)
    }
}

/// Retrato fechado de um quadro.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderSnapshot {
    /// Camera, se houver uma ativa e valida.
    pub camera: Option<CameraSnapshot>,
    /// Entidades desenhaveis, na ordem de iteracao do ECS.
    pub instancias: Vec<InstanceSnapshot>,
}

impl RenderSnapshot {
    /// Indica se nao ha nada a desenhar.
    #[must_use]
    pub fn vazio(&self) -> bool {
        self.instancias.is_empty()
    }
}

/// Extrai o snapshot do mundo.
///
/// Entidades sem [`Visibility`] contam como visiveis: exigir o componente em
/// tudo que se desenha seria ruido, e o padrao util e "aparece".
pub fn extract(world: &mut World) -> RenderSnapshot {
    let camera = extrair_camera(world);

    // A query e coletada antes do laco porque o corpo consulta `Visibility` e
    // `Bounds` por entidade, e isso precisa do mundo de volta. Coletar tuplas de
    // `Copy` mantem tudo no mesmo passe sobre os archetypes.
    let desenhaveis: Vec<(Entity, Transform, Renderable)> =
        world.query::<(Entity, &Transform, &Renderable)>().map(|(e, t, r)| (e, *t, *r)).collect();

    let mut instancias = Vec::with_capacity(desenhaveis.len());
    for (entity, transform, renderable) in desenhaveis {
        if world.get::<Visibility>(entity).is_some_and(|v| !v.visivel) {
            continue;
        }

        let raio_local = world.get::<Bounds>(entity).copied().unwrap_or_default().raio;
        let modelo = transform.matriz();

        instancias.push(InstanceSnapshot {
            entity,
            modelo,
            mesh: renderable.mesh,
            material: renderable.material,
            raio: raio_local * transform.escala_maxima(),
            centro: transform.translacao,
        });
    }

    RenderSnapshot { camera, instancias }
}

/// Resolve a camera ativa, se ela existir e ainda for valida.
///
/// Uma camera apontada por [`CameraAtiva`] que foi destruida, ou que perdeu o
/// `Transform`, simplesmente nao produz camera — em vez de entrar em panico. Um
/// quadro sem camera e um estado legitimo durante carregamento de cena.
fn extrair_camera(world: &mut World) -> Option<CameraSnapshot> {
    let alvo = world.get_resource::<CameraAtiva>().and_then(|c| c.0)?;

    let camera = *world.get::<Camera>(alvo)?;
    let transform = *world.get::<Transform>(alvo)?;

    Some(CameraSnapshot {
        view: transform.matriz().inverse(),
        fov_y: camera.fov_y,
        perto: camera.perto,
        longe: camera.longe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn mundo_com_camera() -> (World, Entity) {
        let mut w = World::new();
        let cam = w.spawn((Camera::default(), Transform::em(0.0, 0.0, 10.0)));
        w.insert_resource(CameraAtiva(Some(cam)));
        (w, cam)
    }

    #[test]
    fn mundo_vazio_produz_snapshot_vazio() {
        let mut w = World::new();
        let s = extract(&mut w);

        assert!(s.vazio());
        assert!(s.camera.is_none());
    }

    #[test]
    fn extrai_entidades_desenhaveis() {
        let (mut w, _) = mundo_com_camera();
        let e = w.spawn((
            Transform::em(1.0, 2.0, 3.0),
            Renderable { mesh: MeshId(7), material: MaterialId(3) },
        ));

        let s = extract(&mut w);

        assert_eq!(s.instancias.len(), 1);
        let i = s.instancias[0];
        assert_eq!(i.entity, e);
        assert_eq!(i.mesh, MeshId(7));
        assert_eq!(i.material, MaterialId(3));
        assert_eq!(i.centro, Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn entidade_sem_renderable_nao_entra() {
        let (mut w, _) = mundo_com_camera();
        w.spawn((Transform::em(0.0, 0.0, 0.0),));

        assert!(extract(&mut w).vazio());
    }

    #[test]
    fn entidade_oculta_nao_entra() {
        let (mut w, _) = mundo_com_camera();
        w.spawn((
            Transform::default(),
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
            Visibility::OCULTA,
        ));

        assert!(extract(&mut w).vazio());
    }

    #[test]
    fn ausencia_de_visibility_conta_como_visivel() {
        let (mut w, _) = mundo_com_camera();
        w.spawn((Transform::default(), Renderable { mesh: MeshId(1), material: MaterialId(1) }));

        assert_eq!(extract(&mut w).instancias.len(), 1);
    }

    #[test]
    fn raio_acompanha_a_escala() {
        let (mut w, _) = mundo_com_camera();
        w.spawn((
            Transform { escala: Vec3::splat(3.0), ..Transform::default() },
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
            Bounds { raio: 2.0 },
        ));

        assert_eq!(extract(&mut w).instancias[0].raio, 6.0);
    }

    #[test]
    fn raio_padrao_quando_nao_ha_bounds() {
        let (mut w, _) = mundo_com_camera();
        w.spawn((Transform::default(), Renderable { mesh: MeshId(1), material: MaterialId(1) }));

        assert_eq!(extract(&mut w).instancias[0].raio, 1.0);
    }

    #[test]
    fn camera_ativa_produz_view() {
        let (mut w, _) = mundo_com_camera();
        let s = extract(&mut w);

        let cam = s.camera.expect("camera ativa");
        // A view leva o mundo para o espaco da camera: a posicao dela vira
        // a origem.
        assert_eq!(cam.view.transform_point3(Vec3::new(0.0, 0.0, 10.0)), Vec3::ZERO);
    }

    #[test]
    fn camera_destruida_nao_produz_camera() {
        let (mut w, cam) = mundo_com_camera();
        w.despawn(cam);

        assert!(extract(&mut w).camera.is_none(), "camera morta nao pode virar panico nem lixo");
    }

    #[test]
    fn camera_sem_transform_nao_produz_camera() {
        let mut w = World::new();
        let cam = w.spawn((Camera::default(),));
        w.insert_resource(CameraAtiva(Some(cam)));

        assert!(extract(&mut w).camera.is_none());
    }

    #[test]
    fn sem_recurso_de_camera_ativa_nao_produz_camera() {
        let mut w = World::new();
        w.spawn((Camera::default(), Transform::default()));

        assert!(extract(&mut w).camera.is_none());
    }

    // --------------------------------------- o snapshot nao vaza o passado --

    #[test]
    fn entidade_removida_nao_aparece_no_snapshot_seguinte() {
        let (mut w, _) = mundo_com_camera();
        let a = w.spawn((
            Transform::em(1.0, 0.0, 0.0),
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
        ));
        let b = w.spawn((
            Transform::em(2.0, 0.0, 0.0),
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
        ));

        assert_eq!(extract(&mut w).instancias.len(), 2);

        w.despawn(a);
        let s = extract(&mut w);

        assert_eq!(s.instancias.len(), 1);
        assert_eq!(s.instancias[0].entity, b);
    }

    #[test]
    fn snapshot_antigo_sobrevive_a_destruicao_das_entidades() {
        // E o ponto do estagio: o retrato e uma copia, entao a simulacao pode
        // avancar sem invalida-lo.
        let (mut w, _) = mundo_com_camera();
        let e = w.spawn((
            Transform::em(5.0, 0.0, 0.0),
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
        ));

        let antigo = extract(&mut w);
        w.despawn(e);

        assert_eq!(antigo.instancias.len(), 1);
        assert_eq!(antigo.instancias[0].centro, Vec3::new(5.0, 0.0, 0.0));
        assert!(extract(&mut w).vazio());
    }

    #[test]
    fn migrar_de_archetype_nao_duplica_nem_perde() {
        let (mut w, _) = mundo_com_camera();
        let e = w
            .spawn((Transform::default(), Renderable { mesh: MeshId(1), material: MaterialId(1) }));

        // Ganhar um componente move a entidade de archetype.
        w.insert(e, (Bounds { raio: 4.0 },));

        let s = extract(&mut w);
        assert_eq!(s.instancias.len(), 1, "a migracao nao pode duplicar");
        assert_eq!(s.instancias[0].raio, 4.0, "nem perder o componente novo");
    }

    #[test]
    fn perder_renderable_tira_a_entidade_do_snapshot() {
        let (mut w, _) = mundo_com_camera();
        let e = w
            .spawn((Transform::default(), Renderable { mesh: MeshId(1), material: MaterialId(1) }));
        assert_eq!(extract(&mut w).instancias.len(), 1);

        w.remove::<Renderable>(e);
        assert!(extract(&mut w).vazio());
    }

    #[test]
    fn extracao_repetida_produz_o_mesmo_snapshot() {
        let (mut w, _) = mundo_com_camera();
        for i in 0..50 {
            w.spawn((
                Transform::em(i as f32, 0.0, 0.0),
                Renderable { mesh: MeshId(i % 3), material: MaterialId(i % 2) },
            ));
        }

        assert_eq!(extract(&mut w), extract(&mut w));
    }
}
