//! Estagio 3: montar o trabalho de render.
//!
//! Agrupa o que sobrou do culling em lotes que compartilham material e malha, e
//! empacota as matrizes de modelo num vetor contiguo. Cada lote vira **um**
//! desenho instanciado, e nao um por entidade — e o que o criterio de aceite da
//! Fase 4 cobra: numero de draw calls constante, nao proporcional a contagem de
//! objetos (secao 8).
//!
//! # Forma canonica
//!
//! A ordenacao e por `(material, malha, entidade)`. Material primeiro porque
//! trocar material e a mudanca de estado mais cara na GPU; entidade por ultimo
//! como desempate estavel.
//!
//! Isso torna o resultado **canonico**: dois mundos com o mesmo conteudo logico
//! produzem o mesmo trabalho de render, ainda que a extracao os tenha
//! percorrido em ordens diferentes. E a garantia que o [`extract`] sozinho nao
//! da, e por isso a ordenacao vive aqui.
//!
//! [`extract`]: crate::extract

use glam::Mat4;

use crate::componentes::{MaterialId, MeshId};
use crate::prepare::PreparedFrame;

/// Dados de uma instancia no formato que o shader espera.
///
/// `repr(C)` porque o layout precisa casar com o do buffer de vertices por
/// instancia. Uma matriz `4x4` de `f32` em ordem de coluna e o que tanto WGSL
/// quanto HLSL leem sem conversao.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct InstanceData {
    /// Matriz de modelo, em ordem de coluna.
    pub modelo: [f32; 16],
}

impl InstanceData {
    /// Tamanho em bytes de uma instancia.
    pub const TAMANHO: u64 = 64;

    /// Constroi a partir de uma matriz.
    #[must_use]
    pub fn de_matriz(m: Mat4) -> Self {
        Self { modelo: m.to_cols_array() }
    }
}

/// Um desenho instanciado.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawBatch {
    /// Malha a desenhar.
    pub mesh: MeshId,
    /// Material com que desenhar.
    pub material: MaterialId,
    /// Primeira instancia deste lote dentro de [`RenderWork::instancias`].
    pub primeira: u32,
    /// Quantas instancias.
    pub quantidade: u32,
}

impl DrawBatch {
    /// Faixa de instancias, no formato que a RHI recebe.
    #[must_use]
    pub fn faixa(&self) -> std::ops::Range<u32> {
        self.primeira..self.primeira + self.quantidade
    }
}

/// Tudo o que o estagio de render precisa, e nada mais.
///
/// Nao ha referencia ao mundo, ao snapshot nem ao quadro preparado: o render
/// consome isto e so isto.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RenderWork {
    /// Visao combinada com projecao.
    pub view_proj: Mat4,
    /// Matrizes de modelo, agrupadas por lote e contiguas.
    pub instancias: Vec<InstanceData>,
    /// Lotes, em ordem canonica.
    pub lotes: Vec<DrawBatch>,
}

impl RenderWork {
    /// Quantos desenhos serao emitidos.
    #[must_use]
    pub fn draw_calls(&self) -> usize {
        self.lotes.len()
    }

    /// Bytes que precisam ir para o buffer de instancias.
    #[must_use]
    pub fn bytes_de_instancia(&self) -> u64 {
        self.instancias.len() as u64 * InstanceData::TAMANHO
    }

    /// Indica se nao ha nada a desenhar.
    #[must_use]
    pub fn vazio(&self) -> bool {
        self.lotes.is_empty()
    }
}

/// Monta o trabalho de render a partir do quadro preparado.
#[must_use]
pub fn queue(frame: &PreparedFrame) -> RenderWork {
    let mut ordenadas: Vec<_> = frame.visiveis.iter().collect();
    ordenadas.sort_unstable_by_key(|i| (i.material, i.mesh, i.entity));

    let mut instancias = Vec::with_capacity(ordenadas.len());
    let mut lotes: Vec<DrawBatch> = Vec::new();

    for i in ordenadas {
        instancias.push(InstanceData::de_matriz(i.modelo));

        // Como a lista esta ordenada, entidades do mesmo par sao vizinhas: basta
        // olhar o ultimo lote.
        match lotes.last_mut() {
            Some(ultimo) if ultimo.mesh == i.mesh && ultimo.material == i.material => {
                ultimo.quantidade += 1;
            }
            _ => lotes.push(DrawBatch {
                mesh: i.mesh,
                material: i.material,
                primeira: instancias.len() as u32 - 1,
                quantidade: 1,
            }),
        }
    }

    RenderWork { view_proj: frame.view_proj, instancias, lotes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::InstanceSnapshot;
    use glam::Vec3;
    use krateus_ecs::Entity;
    use std::num::NonZeroU32;

    fn inst(n: u32, mesh: u32, material: u32) -> InstanceSnapshot {
        InstanceSnapshot {
            entity: Entity::from_raw(n, NonZeroU32::new(1).expect("nao nulo")),
            modelo: Mat4::from_translation(Vec3::splat(n as f32)),
            mesh: MeshId(mesh),
            material: MaterialId(material),
            raio: 1.0,
            centro: Vec3::ZERO,
        }
    }

    fn quadro(visiveis: Vec<InstanceSnapshot>) -> PreparedFrame {
        PreparedFrame { view_proj: Mat4::IDENTITY, visiveis, descartadas: 0 }
    }

    #[test]
    fn quadro_vazio_nao_gera_lote() {
        let w = queue(&quadro(vec![]));

        assert!(w.vazio());
        assert_eq!(w.draw_calls(), 0);
        assert_eq!(w.bytes_de_instancia(), 0);
    }

    #[test]
    fn instancias_do_mesmo_par_viram_um_lote() {
        let w = queue(&quadro(vec![inst(0, 1, 1), inst(1, 1, 1), inst(2, 1, 1)]));

        assert_eq!(w.draw_calls(), 1, "tres objetos, um desenho");
        assert_eq!(w.lotes[0].quantidade, 3);
        assert_eq!(w.lotes[0].faixa(), 0..3);
    }

    #[test]
    fn pares_distintos_viram_lotes_distintos() {
        let w = queue(&quadro(vec![inst(0, 1, 1), inst(1, 2, 1), inst(2, 1, 2)]));

        assert_eq!(w.draw_calls(), 3);
    }

    #[test]
    fn malhas_diferentes_no_mesmo_material_nao_se_misturam() {
        let w = queue(&quadro(vec![inst(0, 1, 5), inst(1, 2, 5), inst(2, 1, 5)]));

        assert_eq!(w.draw_calls(), 2);
        // As duas de malha 1 ficam juntas apesar de nao serem vizinhas na
        // entrada.
        assert_eq!(
            w.lotes[0],
            DrawBatch { mesh: MeshId(1), material: MaterialId(5), primeira: 0, quantidade: 2 }
        );
    }

    #[test]
    fn as_faixas_cobrem_todas_as_instancias_sem_sobreposicao() {
        let w = queue(&quadro(vec![
            inst(0, 1, 1),
            inst(1, 2, 1),
            inst(2, 2, 1),
            inst(3, 1, 2),
            inst(4, 1, 2),
            inst(5, 1, 2),
        ]));

        let mut cobertos: Vec<u32> = Vec::new();
        for lote in &w.lotes {
            cobertos.extend(lote.faixa());
        }
        cobertos.sort_unstable();

        assert_eq!(cobertos, (0..6).collect::<Vec<_>>());
        assert_eq!(w.instancias.len(), 6);
    }

    #[test]
    fn material_ordena_antes_da_malha() {
        // Trocar material e a mudanca de estado mais cara; o agrupamento
        // precisa favorece-la.
        let w = queue(&quadro(vec![inst(0, 9, 2), inst(1, 1, 1)]));

        assert_eq!(w.lotes[0].material, MaterialId(1));
        assert_eq!(w.lotes[1].material, MaterialId(2));
    }

    #[test]
    fn o_resultado_e_canonico_independente_da_ordem_de_entrada() {
        // E a garantia que o extract sozinho nao da: dois historicos diferentes,
        // o mesmo conteudo, o mesmo trabalho de render.
        let direta = quadro(vec![inst(0, 1, 1), inst(1, 2, 2), inst(2, 1, 1)]);
        let invertida = quadro(vec![inst(2, 1, 1), inst(1, 2, 2), inst(0, 1, 1)]);

        assert_eq!(queue(&direta), queue(&invertida));
    }

    #[test]
    fn entidade_desempata_de_forma_estavel() {
        let w = queue(&quadro(vec![inst(5, 1, 1), inst(2, 1, 1), inst(9, 1, 1)]));

        // A ordem das matrizes segue a das entidades: 2, 5, 9.
        let xs: Vec<f32> = w.instancias.iter().map(|i| i.modelo[12]).collect();
        assert_eq!(xs, vec![2.0, 5.0, 9.0]);
    }

    #[test]
    fn a_matriz_vai_em_ordem_de_coluna() {
        let w = queue(&quadro(vec![inst(3, 1, 1)]));

        // Numa matriz em ordem de coluna, a translacao ocupa os indices 12..15.
        assert_eq!(w.instancias[0].modelo[12], 3.0);
        assert_eq!(w.instancias[0].modelo[15], 1.0);
    }

    #[test]
    fn tamanho_declarado_bate_com_o_do_tipo() {
        assert_eq!(size_of::<InstanceData>() as u64, InstanceData::TAMANHO);
    }

    #[test]
    fn draw_calls_nao_crescem_com_a_contagem_de_objetos() {
        // O criterio da secao 8: mil objetos do mesmo par continuam sendo um
        // desenho.
        let muitos: Vec<_> = (0..1000).map(|i| inst(i, 1, 1)).collect();
        let w = queue(&quadro(muitos));

        assert_eq!(w.draw_calls(), 1);
        assert_eq!(w.instancias.len(), 1000);
    }
}
