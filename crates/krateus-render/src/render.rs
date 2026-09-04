//! Estagio 4: consumir o trabalho pela RHI.
//!
//! Este modulo nao decide nada. Ele recebe um [`RenderWork`] pronto e o traduz
//! em chamadas de RHI — adquirir quadro, gravar, submeter, apresentar. Toda
//! escolha de o que desenhar, em que ordem e com qual agrupamento ja foi feita
//! nos estagios anteriores.
//!
//! Essa divisao e o que permite testar o caminho inteiro sem GPU: o
//! `NullRhi` registra o que recebeu, e o teste afirma sobre isso.

use krateus_rhi::{
    BufferId, Color, ColorAttachment, LoadOp, RenderPassDesc, RenderPipelineId, Rhi, RhiError,
    StoreOp,
};

use crate::queue::{InstanceData, RenderWork};

/// Recursos de GPU que o renderer mantem entre quadros.
///
/// Criados uma vez, reusados sempre: recriar pipeline ou buffer por quadro e o
/// tipo de custo que a secao 11 manda evitar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderResources {
    /// Pipeline usado por todos os lotes.
    ///
    /// Um so, por enquanto: material ainda nao seleciona pipeline. Quando
    /// selecionar, o lote passa a carregar qual.
    pub pipeline: RenderPipelineId,
    /// Buffer com os vertices das malhas.
    pub vertices: BufferId,
    /// Buffer com as matrizes de modelo, uma por instancia.
    pub instancias: BufferId,
    /// Quantos vertices cada malha tem.
    ///
    /// Provisorio: com malhas de verdade isto vem do asset, por lote. Hoje todo
    /// lote desenha a mesma contagem.
    pub vertices_por_malha: u32,
}

/// Cor com que o quadro comeca.
pub const FUNDO: Color = Color { r: 0.02, g: 0.02, b: 0.04, a: 1.0 };

/// Desenha um quadro.
///
/// Sem lotes, ainda assim adquire, limpa e apresenta: um quadro sem objetos
/// precisa apagar o anterior, senao a tela congela na ultima imagem.
///
/// # Errors
///
/// Propaga o que a RHI recusar — superficie nao configurada, identificador
/// invalido, ordem de gravacao errada.
pub fn render(
    rhi: &mut dyn Rhi,
    recursos: &RenderResources,
    trabalho: &RenderWork,
) -> Result<(), RhiError> {
    if !trabalho.instancias.is_empty() {
        rhi.write_buffer(recursos.instancias, 0, &bytes_das_instancias(&trabalho.instancias))?;
    }

    let quadro = rhi.acquire_frame()?;
    let encoder = rhi.create_command_encoder("quadro")?;

    let anexos = [ColorAttachment {
        vista: quadro.vista,
        load: LoadOp::Limpar(FUNDO),
        store: StoreOp::Guardar,
    }];
    let passe =
        rhi.begin_render_pass(encoder, &RenderPassDesc { rotulo: "principal", cores: &anexos })?;

    if !trabalho.vazio() {
        rhi.set_pipeline(passe, recursos.pipeline)?;
        rhi.set_vertex_buffer(passe, 0, recursos.vertices)?;
        rhi.set_vertex_buffer(passe, 1, recursos.instancias)?;

        for lote in &trabalho.lotes {
            rhi.draw(passe, 0..recursos.vertices_por_malha, lote.faixa())?;
        }
    }

    rhi.end_render_pass(passe)?;
    let lista = rhi.finish_encoder(encoder)?;
    rhi.submit(&[lista])?;
    rhi.present_frame(quadro)
}

/// Reinterpreta as instancias como bytes.
///
/// `InstanceData` e `repr(C)` e contem apenas `f32`, entao nao tem preenchimento
/// nem invariante que a leitura por bytes possa violar. Um `bytemuck` faria o
/// mesmo com prova em tempo de compilacao; enquanto e um tipo so, a dependencia
/// nao se paga.
fn bytes_das_instancias(instancias: &[InstanceData]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(instancias.len() * InstanceData::TAMANHO as usize);
    for i in instancias {
        for componente in i.modelo {
            bytes.extend_from_slice(&componente.to_le_bytes());
        }
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::componentes::{MaterialId, MeshId};
    use crate::queue::DrawBatch;
    use glam::Mat4;
    use krateus_rhi::{
        BufferDesc, BufferUsage, NullRhi, Operacao, PresentMode, PrimitiveTopology,
        RenderPipelineDesc, ShaderDesc, ShaderSource, SurfaceConfig, TextureFormat,
    };
    use std::num::NonZeroU32;

    const FORMATO: TextureFormat = TextureFormat::Bgra8UnormSrgb;

    fn montar() -> (NullRhi, RenderResources) {
        let mut rhi = NullRhi::new();
        rhi.configure_surface(&SurfaceConfig {
            largura: NonZeroU32::new(800).expect("nao nulo"),
            altura: NonZeroU32::new(600).expect("nao nulo"),
            formato: FORMATO,
            modo: PresentMode::Fifo,
        })
        .expect("superficie");

        let vs = rhi
            .create_shader(&ShaderDesc { rotulo: "vs", fonte: ShaderSource::Wgsl("") })
            .expect("vs");
        let fs = rhi
            .create_shader(&ShaderDesc { rotulo: "fs", fonte: ShaderSource::Wgsl("") })
            .expect("fs");

        let pipeline = rhi
            .create_render_pipeline(&RenderPipelineDesc {
                rotulo: "instanciado",
                vertex: vs,
                vertex_entrada: "vs_main",
                fragment: fs,
                fragment_entrada: "fs_main",
                vertices: &[],
                topologia: PrimitiveTopology::TriangleList,
                formato_alvo: FORMATO,
            })
            .expect("pipeline");

        let vertices = rhi
            .create_buffer(&BufferDesc {
                rotulo: "vertices",
                tamanho: 1024,
                uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
            })
            .expect("vertices");
        let instancias = rhi
            .create_buffer(&BufferDesc {
                rotulo: "instancias",
                tamanho: 64 * 1024,
                uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
            })
            .expect("instancias");

        rhi.limpar_registro();
        (rhi, RenderResources { pipeline, vertices, instancias, vertices_por_malha: 3 })
    }

    fn trabalho_com(lotes: &[(u32, u32, u32)]) -> RenderWork {
        let mut w = RenderWork { view_proj: Mat4::IDENTITY, ..RenderWork::default() };
        let mut primeira = 0;
        for &(mesh, material, quantidade) in lotes {
            for _ in 0..quantidade {
                w.instancias.push(InstanceData::de_matriz(Mat4::IDENTITY));
            }
            w.lotes.push(DrawBatch {
                mesh: MeshId(mesh),
                material: MaterialId(material),
                primeira,
                quantidade,
            });
            primeira += quantidade;
        }
        w
    }

    #[test]
    fn um_lote_vira_um_desenho() {
        let (mut rhi, recursos) = montar();
        render(&mut rhi, &recursos, &trabalho_com(&[(1, 1, 5)])).expect("render");

        assert_eq!(
            rhi.registro(),
            &[
                Operacao::AdquiriuQuadro(0),
                Operacao::AbriuPasse(1),
                Operacao::FixouPipeline,
                Operacao::LigouVertices(0),
                Operacao::LigouVertices(1),
                Operacao::Desenhou(0..3, 0..5),
                Operacao::FechouPasse,
                Operacao::Submeteu(1),
                Operacao::ApresentouQuadro(0),
            ]
        );
    }

    #[test]
    fn cada_lote_desenha_a_propria_faixa() {
        let (mut rhi, recursos) = montar();
        render(&mut rhi, &recursos, &trabalho_com(&[(1, 1, 2), (2, 1, 3)])).expect("render");

        let desenhos: Vec<_> = rhi
            .registro()
            .iter()
            .filter_map(|op| match op {
                Operacao::Desenhou(v, i) => Some((v.clone(), i.clone())),
                _ => None,
            })
            .collect();

        assert_eq!(desenhos, vec![(0..3, 0..2), (0..3, 2..5)]);
    }

    #[test]
    fn quadro_vazio_ainda_limpa_e_apresenta() {
        // Sem isso, a tela congelaria na ultima imagem quando nada e visivel.
        let (mut rhi, recursos) = montar();
        render(&mut rhi, &recursos, &RenderWork::default()).expect("render");

        assert_eq!(
            rhi.registro(),
            &[
                Operacao::AdquiriuQuadro(0),
                Operacao::AbriuPasse(1),
                Operacao::FechouPasse,
                Operacao::Submeteu(1),
                Operacao::ApresentouQuadro(0),
            ]
        );
    }

    #[test]
    fn quadros_sucessivos_nao_travam_a_apresentacao() {
        let (mut rhi, recursos) = montar();
        let w = trabalho_com(&[(1, 1, 1)]);

        for _ in 0..5 {
            render(&mut rhi, &recursos, &w).expect("render");
        }

        let apresentados =
            rhi.registro().iter().filter(|op| matches!(op, Operacao::ApresentouQuadro(_))).count();
        assert_eq!(apresentados, 5);
    }

    #[test]
    fn superficie_nao_configurada_falha_sem_gravar_nada() {
        let mut rhi = NullRhi::new();
        let recursos = RenderResources {
            pipeline: krateus_rhi::RenderPipelineId::from_raw(
                0,
                NonZeroU32::new(1).expect("nao nulo"),
            ),
            vertices: krateus_rhi::BufferId::from_raw(0, NonZeroU32::new(1).expect("nao nulo")),
            instancias: krateus_rhi::BufferId::from_raw(1, NonZeroU32::new(1).expect("nao nulo")),
            vertices_por_malha: 3,
        };

        assert_eq!(
            render(&mut rhi, &recursos, &RenderWork::default()).unwrap_err(),
            RhiError::SuperficieNaoConfigurada
        );
        assert!(rhi.registro().is_empty());
    }

    #[test]
    fn as_matrizes_vao_para_o_buffer_em_ordem_de_coluna() {
        let m = Mat4::from_translation(glam::Vec3::new(1.0, 2.0, 3.0));
        let bytes = bytes_das_instancias(&[InstanceData::de_matriz(m)]);

        assert_eq!(bytes.len() as u64, InstanceData::TAMANHO);
        // A translacao ocupa os componentes 12, 13 e 14.
        let x = f32::from_le_bytes(bytes[48..52].try_into().expect("4 bytes"));
        let y = f32::from_le_bytes(bytes[52..56].try_into().expect("4 bytes"));
        assert_eq!((x, y), (1.0, 2.0));
    }
}
