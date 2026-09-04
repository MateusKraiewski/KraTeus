//! Testes de contrato da RHI, exercitados pelo backend nulo.
//!
//! Cada regra que a `Rhi` enuncia em prosa vira uma verificacao aqui. Um
//! backend real e testado contra este mesmo arquivo: se ele aceitar o que o
//! nulo recusa, ou recusar o que o nulo aceita, a divergencia aparece.
//!
//! Roda sem GPU, entao vale na matriz Windows/Linux do CI.

use std::num::NonZeroU32;

use krateus_rhi::{
    BufferDesc, BufferUsage, Color, ColorAttachment, LoadOp, NullRhi, Operacao, PresentMode,
    PrimitiveTopology, RenderPassDesc, RenderPipelineDesc, Rhi, RhiError, ShaderDesc, ShaderSource,
    StoreOp, SurfaceConfig, TextureFormat, VertexAttribute, VertexFormat, VertexLayout,
};

const FORMATO: TextureFormat = TextureFormat::Bgra8UnormSrgb;

fn nz(v: u32) -> NonZeroU32 {
    NonZeroU32::new(v).expect("dimensao nao nula")
}

fn config(largura: u32, altura: u32) -> SurfaceConfig {
    SurfaceConfig {
        largura: nz(largura),
        altura: nz(altura),
        formato: FORMATO,
        modo: PresentMode::Fifo,
    }
}

/// Monta o minimo para desenhar: superficie, shaders, pipeline e vertices.
fn preparar() -> (NullRhi, krateus_rhi::RenderPipelineId, krateus_rhi::BufferId) {
    let mut rhi = NullRhi::new();
    rhi.configure_surface(&config(1280, 720)).expect("superficie");

    let vs = rhi
        .create_shader(&ShaderDesc { rotulo: "vs", fonte: ShaderSource::Wgsl("// vs") })
        .expect("shader de vertice");
    let fs = rhi
        .create_shader(&ShaderDesc { rotulo: "fs", fonte: ShaderSource::Wgsl("// fs") })
        .expect("shader de fragmento");

    let atributos = [
        VertexAttribute { local: 0, deslocamento: 0, formato: VertexFormat::Float32x3 },
        VertexAttribute { local: 1, deslocamento: 12, formato: VertexFormat::Float32x3 },
    ];
    let layouts = [VertexLayout { passo: 24, atributos: &atributos }];

    let pipeline = rhi
        .create_render_pipeline(&RenderPipelineDesc {
            rotulo: "triangulo",
            vertex: vs,
            vertex_entrada: "vs_main",
            fragment: fs,
            fragment_entrada: "fs_main",
            vertices: &layouts,
            topologia: PrimitiveTopology::TriangleList,
            formato_alvo: FORMATO,
        })
        .expect("pipeline");

    let vertices = rhi
        .create_buffer(&BufferDesc {
            rotulo: "vertices",
            tamanho: 3 * 24,
            uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        })
        .expect("buffer");

    (rhi, pipeline, vertices)
}

// -------------------------------------------------------- o quadro completo --

#[test]
fn o_quadro_do_triangulo_grava_na_ordem_esperada() {
    let (mut rhi, pipeline, vertices) = preparar();
    rhi.write_buffer(vertices, 0, &[0u8; 72]).expect("escrita");
    rhi.limpar_registro();

    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("quadro").expect("encoder");

    let anexos = [ColorAttachment {
        vista: frame.vista,
        load: LoadOp::Limpar(Color::rgb(0.1, 0.2, 0.3)),
        store: StoreOp::Guardar,
    }];
    let passe = rhi
        .begin_render_pass(enc, &RenderPassDesc { rotulo: "principal", cores: &anexos })
        .expect("passe");

    rhi.set_pipeline(passe, pipeline).expect("pipeline");
    rhi.set_vertex_buffer(passe, 0, vertices).expect("vertices");
    rhi.draw(passe, 0..3, 0..1).expect("desenho");
    rhi.end_render_pass(passe).expect("fim do passe");

    let lista = rhi.finish_encoder(enc).expect("lista");
    rhi.submit(&[lista]).expect("submissao");
    rhi.present_frame(frame).expect("apresentacao");

    assert_eq!(
        rhi.registro(),
        &[
            Operacao::AdquiriuQuadro(0),
            Operacao::AbriuPasse(1),
            Operacao::FixouPipeline,
            Operacao::LigouVertices(0),
            Operacao::Desenhou(0..3, 0..1),
            Operacao::FechouPasse,
            Operacao::Submeteu(1),
            Operacao::ApresentouQuadro(0),
        ]
    );
}

#[test]
fn quadros_sucessivos_avancam_o_indice() {
    let (mut rhi, _, _) = preparar();

    for esperado in 0..4 {
        let f = rhi.acquire_frame().expect("quadro");
        assert_eq!(f.indice, esperado);
        rhi.present_frame(f).expect("apresentacao");
    }
}

// ------------------------------------------------------------- superficie --

#[test]
fn adquirir_sem_configurar_falha() {
    let mut rhi = NullRhi::new();
    assert_eq!(rhi.acquire_frame().unwrap_err(), RhiError::SuperficieNaoConfigurada);
    assert_eq!(rhi.surface_format().unwrap_err(), RhiError::SuperficieNaoConfigurada);
}

#[test]
fn dois_quadros_sem_apresentar_falham() {
    // A swapchain tem um numero fixo de imagens; segurar duas trava a
    // apresentacao numa implementacao de verdade.
    let (mut rhi, _, _) = preparar();
    let _primeiro = rhi.acquire_frame().expect("primeiro");

    assert!(matches!(rhi.acquire_frame().unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn apresentar_sem_adquirir_falha() {
    let (mut rhi, _, _) = preparar();
    let f = rhi.acquire_frame().expect("quadro");
    rhi.present_frame(f).expect("apresentacao");

    assert!(matches!(rhi.present_frame(f).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn reconfigurar_invalida_o_quadro_em_voo() {
    // E o que acontece num redimensionamento: a swapchain e recriada e as
    // imagens anteriores deixam de existir.
    let (mut rhi, _, _) = preparar();
    let antigo = rhi.acquire_frame().expect("quadro");

    rhi.configure_surface(&config(800, 600)).expect("reconfiguracao");

    assert!(matches!(rhi.present_frame(antigo).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn profundidade_nao_serve_de_formato_de_superficie() {
    let mut rhi = NullRhi::new();
    let cfg = SurfaceConfig {
        largura: nz(640),
        altura: nz(480),
        formato: TextureFormat::Depth32Float,
        modo: PresentMode::Fifo,
    };

    assert!(matches!(
        rhi.configure_surface(&cfg).unwrap_err(),
        RhiError::FormatoNaoSuportado { .. }
    ));
}

// ---------------------------------------------------------------- recursos --

#[test]
fn buffer_precisa_de_tamanho_e_de_uso() {
    let mut rhi = NullRhi::new();

    let sem_tamanho = BufferDesc { rotulo: "x", tamanho: 0, uso: BufferUsage::VERTEX };
    assert!(matches!(
        rhi.create_buffer(&sem_tamanho).unwrap_err(),
        RhiError::DescritorInvalido { .. }
    ));

    let sem_uso = BufferDesc { rotulo: "x", tamanho: 16, uso: BufferUsage::NENHUM };
    assert!(matches!(rhi.create_buffer(&sem_uso).unwrap_err(), RhiError::DescritorInvalido { .. }));
}

#[test]
fn escrita_exige_copy_dst() {
    let mut rhi = NullRhi::new();
    let b = rhi
        .create_buffer(&BufferDesc { rotulo: "so leitura", tamanho: 16, uso: BufferUsage::VERTEX })
        .expect("buffer");

    assert!(matches!(
        rhi.write_buffer(b, 0, &[0u8; 4]).unwrap_err(),
        RhiError::DescritorInvalido { .. }
    ));
}

#[test]
fn escrita_fora_dos_limites_falha() {
    let mut rhi = NullRhi::new();
    let b = rhi
        .create_buffer(&BufferDesc { rotulo: "pequeno", tamanho: 16, uso: BufferUsage::COPY_DST })
        .expect("buffer");

    assert_eq!(
        rhi.write_buffer(b, 8, &[0u8; 16]).unwrap_err(),
        RhiError::ForaDosLimites { bytes: 16, deslocamento: 8, tamanho: 16 }
    );
    rhi.write_buffer(b, 8, &[0u8; 8]).expect("escrita que cabe exatamente");
}

#[test]
fn identificador_de_recurso_destruido_deixa_de_valer() {
    let mut rhi = NullRhi::new();
    let b = rhi
        .create_buffer(&BufferDesc { rotulo: "x", tamanho: 16, uso: BufferUsage::COPY_DST })
        .expect("buffer");

    rhi.destroy_buffer(b);

    assert!(matches!(
        rhi.write_buffer(b, 0, &[0u8; 4]).unwrap_err(),
        RhiError::IdentificadorInvalido { .. }
    ));
}

#[test]
fn slot_reciclado_nao_ressuscita_identificador_antigo() {
    // A geracao do handle e o que separa "recurso novo no mesmo slot" de
    // "recurso antigo ainda valido".
    let mut rhi = NullRhi::new();
    let desc = BufferDesc { rotulo: "x", tamanho: 16, uso: BufferUsage::COPY_DST };

    let antigo = rhi.create_buffer(&desc).expect("primeiro");
    rhi.destroy_buffer(antigo);
    let novo = rhi.create_buffer(&desc).expect("segundo");

    assert_ne!(antigo, novo);
    assert!(rhi.write_buffer(antigo, 0, &[0u8; 4]).is_err());
    assert!(rhi.write_buffer(novo, 0, &[0u8; 4]).is_ok());
}

#[test]
fn pipeline_com_shader_destruido_falha() {
    let mut rhi = NullRhi::new();
    let vs =
        rhi.create_shader(&ShaderDesc { rotulo: "vs", fonte: ShaderSource::Wgsl("") }).expect("vs");
    let fs =
        rhi.create_shader(&ShaderDesc { rotulo: "fs", fonte: ShaderSource::Wgsl("") }).expect("fs");
    rhi.destroy_shader(fs);

    let desc = RenderPipelineDesc {
        rotulo: "p",
        vertex: vs,
        vertex_entrada: "vs_main",
        fragment: fs,
        fragment_entrada: "fs_main",
        vertices: &[],
        topologia: PrimitiveTopology::TriangleList,
        formato_alvo: FORMATO,
    };

    assert!(matches!(
        rhi.create_render_pipeline(&desc).unwrap_err(),
        RhiError::IdentificadorInvalido { .. }
    ));
}

#[test]
fn destruir_tudo_nao_deixa_recurso_vivo() {
    let (mut rhi, pipeline, vertices) = preparar();
    assert!(rhi.recursos_vivos() > 0);

    rhi.destroy_render_pipeline(pipeline);
    rhi.destroy_buffer(vertices);
    // Os dois shaders do `preparar` continuam vivos; destroi-los exigiria
    // devolve-los, e o pipeline ja os consumiu. Conferir que so eles restam.
    assert_eq!(rhi.recursos_vivos(), 2);
}

// ---------------------------------------------------------- ordem de gravacao --

#[test]
fn desenhar_sem_pipeline_falha() {
    let (mut rhi, _, _) = preparar();
    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");

    assert!(matches!(rhi.draw(passe, 0..3, 0..1).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn desenho_com_faixa_vazia_falha() {
    let (mut rhi, pipeline, _) = preparar();
    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");
    rhi.set_pipeline(passe, pipeline).expect("pipeline");

    assert!(matches!(rhi.draw(passe, 0..0, 0..1).unwrap_err(), RhiError::DescritorInvalido { .. }));
    assert!(matches!(rhi.draw(passe, 0..3, 0..0).unwrap_err(), RhiError::DescritorInvalido { .. }));
}

#[test]
fn dois_passes_abertos_no_mesmo_encoder_falham() {
    let (mut rhi, _, _) = preparar();
    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let desc = RenderPassDesc { rotulo: "p", cores: &anexos };

    let _primeiro = rhi.begin_render_pass(enc, &desc).expect("primeiro passe");
    assert!(matches!(rhi.begin_render_pass(enc, &desc).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn fechar_encoder_com_passe_aberto_falha() {
    let (mut rhi, _, _) = preparar();
    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let _passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");

    assert!(matches!(rhi.finish_encoder(enc).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn gravar_em_passe_fechado_falha() {
    let (mut rhi, pipeline, _) = preparar();
    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");
    rhi.end_render_pass(passe).expect("fim");

    assert!(matches!(rhi.set_pipeline(passe, pipeline).unwrap_err(), RhiError::OrdemInvalida(_)));
    assert!(matches!(rhi.end_render_pass(passe).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn passe_sem_anexo_de_cor_falha() {
    let (mut rhi, _, _) = preparar();
    let enc = rhi.create_command_encoder("e").expect("encoder");

    assert!(matches!(
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &[] }).unwrap_err(),
        RhiError::DescritorInvalido { .. }
    ));
}

#[test]
fn submeter_a_mesma_lista_duas_vezes_falha() {
    let (mut rhi, _, _) = preparar();
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let lista = rhi.finish_encoder(enc).expect("lista");

    rhi.submit(&[lista]).expect("primeira submissao");
    assert!(matches!(rhi.submit(&[lista]).unwrap_err(), RhiError::OrdemInvalida(_)));
}

#[test]
fn submissao_invalida_nao_consome_nenhuma_lista() {
    // Validar tudo antes de marcar qualquer coisa: uma submissao parcial
    // deixaria o estado ambiguo.
    let (mut rhi, _, _) = preparar();
    let enc_a = rhi.create_command_encoder("a").expect("encoder a");
    let boa = rhi.finish_encoder(enc_a).expect("lista boa");

    let enc_b = rhi.create_command_encoder("b").expect("encoder b");
    let ruim = rhi.finish_encoder(enc_b).expect("lista b");
    rhi.submit(&[ruim]).expect("consome a segunda");

    // Agora `ruim` ja foi submetida; a tentativa conjunta precisa falhar sem
    // consumir `boa`.
    assert!(rhi.submit(&[boa, ruim]).is_err());
    rhi.submit(&[boa]).expect("a primeira precisa continuar submetivel");
}

#[test]
fn pipeline_de_formato_diferente_do_passe_falha() {
    // Vulkan e DX12 compilam o pipeline para um formato de alvo especifico.
    let mut rhi = NullRhi::new();
    rhi.configure_surface(&config(640, 480)).expect("superficie");

    let vs =
        rhi.create_shader(&ShaderDesc { rotulo: "vs", fonte: ShaderSource::Wgsl("") }).expect("vs");
    let fs =
        rhi.create_shader(&ShaderDesc { rotulo: "fs", fonte: ShaderSource::Wgsl("") }).expect("fs");

    let pipeline = rhi
        .create_render_pipeline(&RenderPipelineDesc {
            rotulo: "outro formato",
            vertex: vs,
            vertex_entrada: "vs_main",
            fragment: fs,
            fragment_entrada: "fs_main",
            vertices: &[],
            topologia: PrimitiveTopology::TriangleList,
            formato_alvo: TextureFormat::Rgba8UnormSrgb,
        })
        .expect("pipeline");

    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");

    assert!(matches!(
        rhi.set_pipeline(passe, pipeline).unwrap_err(),
        RhiError::DescritorInvalido { .. }
    ));
}

#[test]
fn vertices_precisam_de_uso_vertex() {
    let (mut rhi, pipeline, _) = preparar();
    let so_uniform = rhi
        .create_buffer(&BufferDesc { rotulo: "u", tamanho: 64, uso: BufferUsage::UNIFORM })
        .expect("buffer");

    let frame = rhi.acquire_frame().expect("quadro");
    let enc = rhi.create_command_encoder("e").expect("encoder");
    let anexos =
        [ColorAttachment { vista: frame.vista, load: LoadOp::Carregar, store: StoreOp::Guardar }];
    let passe =
        rhi.begin_render_pass(enc, &RenderPassDesc { rotulo: "p", cores: &anexos }).expect("passe");
    rhi.set_pipeline(passe, pipeline).expect("pipeline");

    assert!(matches!(
        rhi.set_vertex_buffer(passe, 0, so_uniform).unwrap_err(),
        RhiError::DescritorInvalido { .. }
    ));
}

// ------------------------------------------------------------------- forma --

#[test]
fn a_interface_e_objeto_segura() {
    // O `krateus-render` vai segurar `Box<dyn Rhi>` e trocar de backend em
    // tempo de execucao; se a interface deixar de ser objeto-segura, e aqui que
    // se descobre.
    let mut backend: Box<dyn Rhi> = Box::new(NullRhi::new());

    assert_eq!(backend.nome(), "null");
    backend.configure_surface(&config(320, 240)).expect("superficie");
    assert_eq!(backend.surface_format().expect("formato"), FORMATO);
}
