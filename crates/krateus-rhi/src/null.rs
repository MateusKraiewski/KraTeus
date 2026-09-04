//! Backend que nao desenha nada, e valida tudo.
//!
//! Serve a dois propositos, e o segundo e o que justifica escreve-lo antes de
//! haver GPU:
//!
//! 1. **Verificar o contrato no CI.** Toda regra que a [`Rhi`] enuncia em prosa
//!    — passe fechado antes de submeter, pipeline fixado antes de desenhar,
//!    quadro apresentado antes de adquirir outro — vira erro aqui. Um backend
//!    real que quebre a ordem falha nos mesmos testes.
//! 2. **Rodar a engine sem janela.** Testes de simulacao, benchmarks de
//!    extracao e o CI Linux nao precisam de GPU para exercitar o caminho ate a
//!    RHI.
//!
//! Ele tambem registra o que recebeu, em [`NullRhi::registro`], para que um
//! teste possa afirmar *o que* foi gravado, e nao apenas que nada falhou.

use std::ops::Range;

use krateus_core::pool::Pool;

use crate::error::{Result, RhiError};
use crate::rhi::Rhi;
use crate::types::{
    BufferDesc, BufferId, BufferUsage, CommandBufferId, CommandEncoderId, Frame, RenderPassDesc,
    RenderPassId, RenderPipelineDesc, RenderPipelineId, ShaderDesc, ShaderId, SurfaceConfig,
    TextureFormat, TextureViewId, reetiqueta,
};

/// Uma operacao observada pelo backend nulo.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Operacao {
    /// Superficie configurada com esta largura e altura.
    ConfigurouSuperficie(u32, u32),
    /// Quadro adquirido.
    AdquiriuQuadro(u32),
    /// Quadro apresentado.
    ApresentouQuadro(u32),
    /// Passe aberto com este numero de anexos de cor.
    AbriuPasse(usize),
    /// Pipeline fixado.
    FixouPipeline,
    /// Buffer de vertices ligado a este slot.
    LigouVertices(u32),
    /// Desenho: faixa de vertices e faixa de instancias.
    Desenhou(Range<u32>, Range<u32>),
    /// Passe fechado.
    FechouPasse,
    /// Listas submetidas.
    Submeteu(usize),
}

#[derive(Debug)]
struct BufferNulo {
    tamanho: u64,
    uso: BufferUsage,
}

#[derive(Debug)]
struct ShaderNulo;

#[derive(Debug)]
struct PipelineNulo {
    formato_alvo: TextureFormat,
}

#[derive(Debug)]
struct EncoderNulo {
    passe_aberto: bool,
    fechado: bool,
}

#[derive(Debug)]
struct PasseNulo {
    fechado: bool,
    tem_pipeline: bool,
}

#[derive(Debug)]
struct ListaNula {
    submetida: bool,
}

/// Backend sem GPU.
#[derive(Debug, Default)]
pub struct NullRhi {
    buffers: Pool<BufferNulo>,
    shaders: Pool<ShaderNulo>,
    pipelines: Pool<PipelineNulo>,
    encoders: Pool<EncoderNulo>,
    passes: Pool<PasseNulo>,
    listas: Pool<ListaNula>,

    config: Option<SurfaceConfig>,
    quadro_em_voo: Option<Frame>,
    proximo_indice: u32,

    registro: Vec<Operacao>,
}

impl NullRhi {
    /// Cria o backend.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Operacoes observadas, na ordem em que chegaram.
    #[must_use]
    pub fn registro(&self) -> &[Operacao] {
        &self.registro
    }

    /// Esquece o que foi observado, preservando os recursos criados.
    pub fn limpar_registro(&mut self) {
        self.registro.clear();
    }

    /// Quantos recursos ainda estao vivos.
    ///
    /// Serve para um teste afirmar que nada vazou depois de destruir tudo.
    #[must_use]
    pub fn recursos_vivos(&self) -> usize {
        self.buffers.len() + self.shaders.len() + self.pipelines.len()
    }

    fn anota(&mut self, op: Operacao) {
        self.registro.push(op);
    }

    fn invalido(tipo: &'static str) -> RhiError {
        RhiError::IdentificadorInvalido { tipo }
    }
}

impl Rhi for NullRhi {
    fn nome(&self) -> &str {
        "null"
    }

    // ------------------------------------------------------------ recursos --

    fn create_buffer(&mut self, desc: &BufferDesc<'_>) -> Result<BufferId> {
        if desc.tamanho == 0 {
            return Err(RhiError::DescritorInvalido { tipo: "buffer", motivo: "tamanho zero" });
        }
        if desc.uso.vazio() {
            // Vulkan e DX12 escolhem o tipo de memoria a partir do uso; sem uso
            // declarado nao ha o que escolher.
            return Err(RhiError::DescritorInvalido {
                tipo: "buffer",
                motivo: "nenhum uso declarado",
            });
        }

        let h = self.buffers.insert(BufferNulo { tamanho: desc.tamanho, uso: desc.uso });
        Ok(reetiqueta(h))
    }

    fn write_buffer(&mut self, buffer: BufferId, deslocamento: u64, dados: &[u8]) -> Result<()> {
        let alvo = self.buffers.get(reetiqueta(buffer)).ok_or_else(|| Self::invalido("buffer"))?;

        if !alvo.uso.contem(BufferUsage::COPY_DST) {
            return Err(RhiError::DescritorInvalido {
                tipo: "buffer",
                motivo: "escrita em buffer sem COPY_DST",
            });
        }

        let bytes = dados.len() as u64;
        if deslocamento.saturating_add(bytes) > alvo.tamanho {
            return Err(RhiError::ForaDosLimites { bytes, deslocamento, tamanho: alvo.tamanho });
        }
        Ok(())
    }

    fn destroy_buffer(&mut self, buffer: BufferId) {
        self.buffers.remove(reetiqueta(buffer));
    }

    fn create_shader(&mut self, _desc: &ShaderDesc<'_>) -> Result<ShaderId> {
        Ok(reetiqueta(self.shaders.insert(ShaderNulo)))
    }

    fn destroy_shader(&mut self, shader: ShaderId) {
        self.shaders.remove(reetiqueta(shader));
    }

    fn create_render_pipeline(
        &mut self,
        desc: &RenderPipelineDesc<'_>,
    ) -> Result<RenderPipelineId> {
        if !self.shaders.contains(reetiqueta(desc.vertex))
            || !self.shaders.contains(reetiqueta(desc.fragment))
        {
            return Err(Self::invalido("shader"));
        }
        if desc.formato_alvo.e_profundidade() {
            return Err(RhiError::DescritorInvalido {
                tipo: "pipeline",
                motivo: "formato de profundidade como alvo de cor",
            });
        }

        let h = self.pipelines.insert(PipelineNulo { formato_alvo: desc.formato_alvo });
        Ok(reetiqueta(h))
    }

    fn destroy_render_pipeline(&mut self, pipeline: RenderPipelineId) {
        self.pipelines.remove(reetiqueta(pipeline));
    }

    // ----------------------------------------------------------- superficie --

    fn configure_surface(&mut self, config: &SurfaceConfig) -> Result<()> {
        if config.formato.e_profundidade() {
            return Err(RhiError::FormatoNaoSuportado { formato: config.formato });
        }

        // Reconfigurar invalida qualquer quadro em voo, como a recriacao de
        // swapchain faz no Vulkan.
        self.quadro_em_voo = None;
        self.config = Some(*config);
        self.anota(Operacao::ConfigurouSuperficie(config.largura.get(), config.altura.get()));
        Ok(())
    }

    fn surface_format(&self) -> Result<TextureFormat> {
        self.config.map(|c| c.formato).ok_or(RhiError::SuperficieNaoConfigurada)
    }

    fn acquire_frame(&mut self) -> Result<Frame> {
        if self.config.is_none() {
            return Err(RhiError::SuperficieNaoConfigurada);
        }
        if self.quadro_em_voo.is_some() {
            // A swapchain tem um numero fixo de imagens; segurar duas sem
            // apresentar trava a apresentacao de verdade.
            return Err(RhiError::OrdemInvalida(
                "adquirir outro quadro antes de apresentar o corrente",
            ));
        }

        let indice = self.proximo_indice;
        self.proximo_indice = self.proximo_indice.wrapping_add(1);

        // A vista e sintetica: nao ha textura de verdade por tras.
        let frame = Frame {
            vista: TextureViewId::from_raw(
                indice,
                std::num::NonZeroU32::new(1).expect("constante nao nula"),
            ),
            indice,
        };
        self.quadro_em_voo = Some(frame);
        self.anota(Operacao::AdquiriuQuadro(indice));
        Ok(frame)
    }

    fn present_frame(&mut self, frame: Frame) -> Result<()> {
        match self.quadro_em_voo {
            Some(corrente) if corrente == frame => {
                self.quadro_em_voo = None;
                self.anota(Operacao::ApresentouQuadro(frame.indice));
                Ok(())
            }
            Some(_) => Err(RhiError::OrdemInvalida("apresentar um quadro que nao e o corrente")),
            None => Err(RhiError::OrdemInvalida("apresentar sem ter adquirido")),
        }
    }

    // -------------------------------------------------------------- comandos --

    fn create_command_encoder(&mut self, _rotulo: &str) -> Result<CommandEncoderId> {
        let h = self.encoders.insert(EncoderNulo { passe_aberto: false, fechado: false });
        Ok(reetiqueta(h))
    }

    fn begin_render_pass(
        &mut self,
        encoder: CommandEncoderId,
        desc: &RenderPassDesc<'_>,
    ) -> Result<RenderPassId> {
        if desc.cores.is_empty() {
            return Err(RhiError::DescritorInvalido {
                tipo: "passe",
                motivo: "nenhum anexo de cor",
            });
        }

        let enc =
            self.encoders.get_mut(reetiqueta(encoder)).ok_or_else(|| Self::invalido("encoder"))?;

        if enc.fechado {
            return Err(RhiError::OrdemInvalida("gravar num encoder ja fechado"));
        }
        if enc.passe_aberto {
            return Err(RhiError::OrdemInvalida("abrir passe com outro ja aberto"));
        }
        enc.passe_aberto = true;

        let quantidade = desc.cores.len();
        let h = self.passes.insert(PasseNulo { fechado: false, tem_pipeline: false });
        self.anota(Operacao::AbriuPasse(quantidade));
        Ok(reetiqueta(h))
    }

    fn set_pipeline(&mut self, passe: RenderPassId, pipeline: RenderPipelineId) -> Result<()> {
        let alvo_do_pipeline = self
            .pipelines
            .get(reetiqueta(pipeline))
            .ok_or_else(|| Self::invalido("pipeline"))?
            .formato_alvo;

        // Vulkan e DX12 exigem que o formato do alvo declarado no pipeline case
        // com o do passe: o estado foi compilado para aquele formato. Aqui as
        // unicas vistas vem da superficie, entao e contra ela que se compara.
        if let Some(config) = self.config
            && alvo_do_pipeline != config.formato
        {
            return Err(RhiError::DescritorInvalido {
                tipo: "pipeline",
                motivo: "formato de alvo diferente do formato do passe",
            });
        }

        let p = self.passes.get_mut(reetiqueta(passe)).ok_or_else(|| Self::invalido("passe"))?;
        if p.fechado {
            return Err(RhiError::OrdemInvalida("gravar num passe ja fechado"));
        }
        p.tem_pipeline = true;

        self.anota(Operacao::FixouPipeline);
        Ok(())
    }

    fn set_vertex_buffer(
        &mut self,
        passe: RenderPassId,
        slot: u32,
        buffer: BufferId,
    ) -> Result<()> {
        let alvo = self.buffers.get(reetiqueta(buffer)).ok_or_else(|| Self::invalido("buffer"))?;
        if !alvo.uso.contem(BufferUsage::VERTEX) {
            return Err(RhiError::DescritorInvalido {
                tipo: "buffer",
                motivo: "ligado como vertice sem uso VERTEX",
            });
        }

        let p = self.passes.get(reetiqueta(passe)).ok_or_else(|| Self::invalido("passe"))?;
        if p.fechado {
            return Err(RhiError::OrdemInvalida("gravar num passe ja fechado"));
        }

        self.anota(Operacao::LigouVertices(slot));
        Ok(())
    }

    fn draw(
        &mut self,
        passe: RenderPassId,
        vertices: Range<u32>,
        instancias: Range<u32>,
    ) -> Result<()> {
        if vertices.is_empty() || instancias.is_empty() {
            return Err(RhiError::DescritorInvalido {
                tipo: "draw",
                motivo: "faixa de vertices ou de instancias vazia",
            });
        }

        let p = self.passes.get(reetiqueta(passe)).ok_or_else(|| Self::invalido("passe"))?;
        if p.fechado {
            return Err(RhiError::OrdemInvalida("desenhar num passe ja fechado"));
        }
        if !p.tem_pipeline {
            return Err(RhiError::OrdemInvalida("desenhar sem pipeline fixado"));
        }

        self.anota(Operacao::Desenhou(vertices, instancias));
        Ok(())
    }

    fn end_render_pass(&mut self, passe: RenderPassId) -> Result<()> {
        let p = self.passes.get_mut(reetiqueta(passe)).ok_or_else(|| Self::invalido("passe"))?;
        if p.fechado {
            return Err(RhiError::OrdemInvalida("fechar um passe ja fechado"));
        }
        p.fechado = true;

        // O passe pertence a exatamente um encoder; como so ha um aberto por
        // vez, fechar aqui libera aquele.
        for (_, enc) in self.encoders.iter_mut() {
            enc.passe_aberto = false;
        }

        self.anota(Operacao::FechouPasse);
        Ok(())
    }

    fn finish_encoder(&mut self, encoder: CommandEncoderId) -> Result<CommandBufferId> {
        let enc =
            self.encoders.get_mut(reetiqueta(encoder)).ok_or_else(|| Self::invalido("encoder"))?;
        if enc.fechado {
            return Err(RhiError::OrdemInvalida("fechar um encoder ja fechado"));
        }
        if enc.passe_aberto {
            return Err(RhiError::OrdemInvalida("fechar encoder com passe aberto"));
        }
        enc.fechado = true;

        Ok(reetiqueta(self.listas.insert(ListaNula { submetida: false })))
    }

    fn submit(&mut self, listas: &[CommandBufferId]) -> Result<()> {
        // Valida tudo antes de marcar qualquer coisa: uma submissao parcial
        // deixaria o estado ambiguo.
        for &id in listas {
            let l = self.listas.get(reetiqueta(id)).ok_or_else(|| Self::invalido("lista"))?;
            if l.submetida {
                return Err(RhiError::OrdemInvalida("submeter a mesma lista duas vezes"));
            }
        }
        for &id in listas {
            if let Some(l) = self.listas.get_mut(reetiqueta(id)) {
                l.submetida = true;
            }
        }

        self.anota(Operacao::Submeteu(listas.len()));
        Ok(())
    }
}
