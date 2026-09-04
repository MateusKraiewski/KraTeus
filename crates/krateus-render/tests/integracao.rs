//! O caminho inteiro, do mundo logico ao comando de GPU, sem janela.
//!
//! Amarra os quatro estagios ao `Schedule` e confere o que sai do outro lado. E
//! o teste que a Fase 3 nao pode fazer — ele nao desenha nada — e que a costura
//! entre simulacao e render pode fazer inteira.

use std::sync::{Arc, Mutex};

use krateus_ecs::{Query, Schedule, World};
use krateus_render::{
    Bounds, Camera, CameraAtiva, MaterialId, MeshId, PreparedFrame, RenderSnapshot, RenderWork,
    Renderable, Transform, Visibility, extract, prepare, queue,
};
use krateus_rhi::{
    BufferDesc, BufferUsage, NullRhi, Operacao, PresentMode, PrimitiveTopology, RenderPipelineDesc,
    Rhi, ShaderDesc, ShaderSource, SurfaceConfig, TextureFormat,
};

const FORMATO: TextureFormat = TextureFormat::Bgra8UnormSrgb;
const LARGURA: u32 = 1280;
const ALTURA: u32 = 720;

struct Velocidade(f32);

/// Sistema de simulacao: move o que tem velocidade.
fn mover(mut q: Query<(&mut Transform, &Velocidade)>) {
    for (t, v) in q.iter() {
        t.translacao.x += v.0;
    }
}

fn montar_rhi() -> (NullRhi, krateus_render::RenderResources) {
    let mut rhi = NullRhi::new();
    rhi.configure_surface(&SurfaceConfig {
        largura: LARGURA.try_into().expect("nao nulo"),
        altura: ALTURA.try_into().expect("nao nulo"),
        formato: FORMATO,
        modo: PresentMode::Fifo,
    })
    .expect("superficie");

    let vs =
        rhi.create_shader(&ShaderDesc { rotulo: "vs", fonte: ShaderSource::Wgsl("") }).expect("vs");
    let fs =
        rhi.create_shader(&ShaderDesc { rotulo: "fs", fonte: ShaderSource::Wgsl("") }).expect("fs");
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
            tamanho: 4096,
            uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        })
        .expect("vertices");
    let instancias = rhi
        .create_buffer(&BufferDesc {
            rotulo: "instancias",
            tamanho: 1024 * 1024,
            uso: BufferUsage::VERTEX | BufferUsage::COPY_DST,
        })
        .expect("instancias");

    rhi.limpar_registro();
    (rhi, krateus_render::RenderResources { pipeline, vertices, instancias, vertices_por_malha: 3 })
}

fn mundo_com_cena() -> World {
    let mut w = World::new();

    let cam = w.spawn((Camera::default(), Transform::em(0.0, 0.0, 30.0)));
    w.insert_resource(CameraAtiva(Some(cam)));

    // Tres pares (malha, material), quatro objetos de cada, todos a frente.
    for par in 0..3_u32 {
        for i in 0..4_u32 {
            w.spawn((
                Transform::em(par as f32 * 2.0 - 2.0, i as f32 - 1.5, 0.0),
                Renderable { mesh: MeshId(par), material: MaterialId(par) },
                Bounds { raio: 0.5 },
                Velocidade(0.0),
            ));
        }
    }
    w
}

fn aspecto() -> f32 {
    LARGURA as f32 / ALTURA as f32
}

// -------------------------------------------------------- o caminho inteiro --

#[test]
fn do_mundo_ao_comando_de_gpu() {
    let mut world = mundo_com_cena();
    let (mut rhi, recursos) = montar_rhi();

    let snapshot = extract(&mut world);
    let quadro = prepare(&snapshot, aspecto());
    let trabalho = queue(&quadro);
    krateus_render::render(&mut rhi, &recursos, &trabalho).expect("render");

    assert_eq!(snapshot.instancias.len(), 12);
    assert_eq!(quadro.visiveis.len(), 12, "todos estao no campo de visao");
    assert_eq!(trabalho.draw_calls(), 3, "doze objetos, tres pares, tres desenhos");

    let desenhos =
        rhi.registro().iter().filter(|op| matches!(op, Operacao::Desenhou(_, _))).count();
    assert_eq!(desenhos, 3);
}

#[test]
fn a_extracao_roda_como_sistema_exclusivo_depois_da_simulacao() {
    // A forma correta de ligar a extracao ao scheduler: um sistema exclusivo,
    // que forma fronteira e nao mente sobre o proprio acesso.
    let capturado: Arc<Mutex<Option<RenderSnapshot>>> = Arc::new(Mutex::new(None));

    let destino = Arc::clone(&capturado);
    let extrair = move |world: &mut World| {
        *destino.lock().expect("sem envenenamento") = Some(extract(world));
    };

    let mut world = mundo_com_cena();
    for (t, _) in world.query::<(&mut Transform, &Velocidade)>() {
        t.translacao.x = 0.0;
    }
    for (_, v) in world.query::<(&Transform, &mut Velocidade)>() {
        v.0 = 1.0;
    }

    let mut schedule = Schedule::new();
    schedule.add_stage("simulacao").add_stage("extracao");
    schedule.add_system("simulacao", mover);
    schedule.add_exclusive_system("extracao", extrair);
    schedule.initialize(&mut world);

    schedule.run(&mut world);

    let snapshot = capturado.lock().expect("sem envenenamento").take().expect("extraido");
    assert_eq!(snapshot.instancias.len(), 12);

    // A extracao viu o mundo **depois** de `mover`: cada objeto andou uma
    // unidade em x.
    let deslocados = snapshot.instancias.iter().filter(|i| (i.centro.x - 1.0).abs() < 1e-5).count();
    assert_eq!(deslocados, 12, "a extracao precisa ver o resultado da simulacao");
}

#[test]
fn a_extracao_forma_fronteira_no_agrupamento() {
    let extrair = |world: &mut World| {
        let _ = extract(world);
    };

    let mut world = mundo_com_cena();
    let mut schedule = Schedule::new();
    schedule.add_stage("quadro");
    schedule.add_system("quadro", mover);
    schedule.add_exclusive_system("quadro", extrair);
    schedule.initialize(&mut world);

    let subetapas = schedule.batches("quadro").expect("etapa existe");
    assert_eq!(subetapas.len(), 2, "a extracao nao pode paralelizar com a simulacao");
    assert_eq!(subetapas[1], vec![1]);
}

// ------------------------------------------------ o snapshot nao vaza nada --

#[test]
fn entidade_destruida_apos_a_extracao_nao_invalida_o_quadro() {
    let mut world = mundo_com_cena();
    let snapshot = extract(&mut world);

    let vivas: Vec<_> = world.entities().iter().collect();
    for e in vivas {
        world.despawn(e);
    }

    // O retrato e uma copia: continua desenhavel com o mundo vazio.
    let quadro = prepare(&snapshot, aspecto());
    let trabalho = queue(&quadro);

    let (mut rhi, recursos) = montar_rhi();
    krateus_render::render(&mut rhi, &recursos, &trabalho).expect("render");

    assert_eq!(trabalho.draw_calls(), 3);
    assert!(world.is_empty());
}

#[test]
fn entidade_removida_some_do_quadro_seguinte() {
    let mut world = mundo_com_cena();
    assert_eq!(quadro_de(&mut world).draw_calls(), 3);

    // Remove os quatro objetos do primeiro par.
    let alvos: Vec<_> = world
        .query::<(krateus_ecs::Entity, &Renderable)>()
        .filter(|(_, r)| r.mesh == MeshId(0))
        .map(|(e, _)| e)
        .collect();
    for e in alvos {
        world.despawn(e);
    }

    let trabalho = quadro_de(&mut world);
    assert_eq!(trabalho.draw_calls(), 2);
    assert_eq!(trabalho.instancias.len(), 8);
}

#[test]
fn ocultar_tira_do_quadro_sem_destruir_a_entidade() {
    let mut world = mundo_com_cena();
    let alvos: Vec<_> = world
        .query::<(krateus_ecs::Entity, &Renderable)>()
        .filter(|(_, r)| r.mesh == MeshId(1))
        .map(|(e, _)| e)
        .collect();

    let quantas = alvos.len();
    for e in alvos {
        world.insert(e, (Visibility::OCULTA,));
    }

    let trabalho = quadro_de(&mut world);
    assert_eq!(trabalho.draw_calls(), 2);
    assert_eq!(world.len(), 13, "as entidades continuam existindo: 12 + camera");
    assert_eq!(quantas, 4);
}

#[test]
fn migrar_de_archetype_nao_duplica_no_quadro() {
    let mut world = mundo_com_cena();
    let antes = quadro_de(&mut world).instancias.len();

    // Ganhar um componente move a entidade de archetype.
    let alvos: Vec<_> =
        world.query::<(krateus_ecs::Entity, &Renderable)>().map(|(e, _)| e).collect();
    for e in alvos {
        world.insert(e, (Visibility::VISIVEL,));
    }

    assert_eq!(quadro_de(&mut world).instancias.len(), antes);
}

// ------------------------------------------------------------ determinismo --

#[test]
fn o_mesmo_mundo_produz_o_mesmo_trabalho() {
    let mut a = mundo_com_cena();
    let mut b = mundo_com_cena();

    assert_eq!(quadro_de(&mut a), quadro_de(&mut b));
}

#[test]
fn historicos_diferentes_produzem_o_mesmo_trabalho() {
    // A forma canonica que o `queue` garante: dois mundos com o mesmo conteudo
    // logico, montados em ordens diferentes, geram o mesmo trabalho de render.
    let mut direto = World::new();
    let cam = direto.spawn((Camera::default(), Transform::em(0.0, 0.0, 30.0)));
    direto.insert_resource(CameraAtiva(Some(cam)));
    for par in 0..3_u32 {
        direto.spawn((
            Transform::em(par as f32, 0.0, 0.0),
            Renderable { mesh: MeshId(par), material: MaterialId(par) },
            Bounds { raio: 0.5 },
        ));
    }

    let mut torto = World::new();
    let cam = torto.spawn((Camera::default(), Transform::em(0.0, 0.0, 30.0)));
    torto.insert_resource(CameraAtiva(Some(cam)));
    // Cria em ordem invertida, e ainda cria e destroi uma entidade no meio para
    // embaralhar os slots.
    let descartavel = torto
        .spawn((Transform::default(), Renderable { mesh: MeshId(99), material: MaterialId(99) }));
    for par in (0..3_u32).rev() {
        torto.spawn((
            Transform::em(par as f32, 0.0, 0.0),
            Renderable { mesh: MeshId(par), material: MaterialId(par) },
            Bounds { raio: 0.5 },
        ));
    }
    torto.despawn(descartavel);

    let a = quadro_de(&mut direto);
    let b = quadro_de(&mut torto);

    assert_eq!(a.lotes, b.lotes, "os lotes precisam ser canonicos");
    assert_eq!(a.instancias, b.instancias);
}

#[test]
fn muitos_objetos_do_mesmo_par_continuam_um_desenho() {
    // O criterio da secao 8, atravessando a costura inteira.
    let mut world = World::new();
    let cam = world.spawn((Camera::default(), Transform::em(0.0, 0.0, 200.0)));
    world.insert_resource(CameraAtiva(Some(cam)));

    for i in 0..5_000 {
        world.spawn((
            Transform::em((i % 50) as f32 - 25.0, (i / 50) as f32 - 50.0, 0.0),
            Renderable { mesh: MeshId(1), material: MaterialId(1) },
            Bounds { raio: 0.4 },
        ));
    }

    let trabalho = quadro_de(&mut world);
    assert_eq!(trabalho.draw_calls(), 1, "cinco mil objetos, um desenho");
    assert!(trabalho.instancias.len() > 1000, "e o culling nao pode ter comido tudo");
}

// ----------------------------------------------------------------- auxiliar --

fn quadro_de(world: &mut World) -> RenderWork {
    let snapshot = extract(world);
    let preparado: PreparedFrame = prepare(&snapshot, aspecto());
    queue(&preparado)
}
