# Registro de Riscos — KraTeus Engine

Riscos com impacto em cronograma ou arquitetura. Um risco so sai daqui quando
for eliminado ou aceito de forma explicita, com a decisao registrada.

---

## R01 — Smart App Control bloqueia binarios de desenvolvimento

**Status:** aberto — aceito para a Fase 2, bloqueante a partir da Fase 3
**Registrado em:** 2026-09-03
**Detalhe tecnico completo:** [AMBIENTE.md](AMBIENTE.md)

### O que acontece

O Smart App Control esta em modo enforce nesta maquina e bloqueia executaveis e
DLLs que nao reconhece por reputacao. Como todo binario recem-compilado e
desconhecido por definicao, o bloqueio atinge:

- **DLLs de proc-macro** carregadas pelo `rustc` — `serde`, `thiserror`,
  `tracing` e praticamente todo o ecossistema.
- **Build scripts** (`build-script-build.exe`), incluindo o de `num-traits`,
  dependencia de `criterion`.
- **Binarios de teste e executaveis do projeto**, de forma imprevisivel e
  **mutavel no tempo**: `krateus_core-8536b4e19a308749.exe` rodou os 19 testes
  com sucesso e, minutos depois, passou a ser bloqueado — mesmo hash, mesmo
  caminho. O SAC consulta reputacao na nuvem de forma assincrona e revoga o que
  ja havia liberado. Forcar hash novo (`-C metadata=`) nao contorna.

Consequencia: `cargo test` nao e confiavel nesta maquina, nem para codigo puro de
CPU. O laco local viavel e `cargo check` + `cargo fmt`.

### Mitigacao aplicada

`target/` redirecionado para `%LOCALAPPDATA%` via `.cargo/config.toml` local, nao
versionado. Resolve proc-macros e a maior parte dos build scripts. Nao resolve
`criterion` nem os binarios gerados.

### Impacto por fase

| Fase | Impacto | Bloqueante? |
|---|---|---|
| 2 — ECS, jobs, loop | `cargo check` local funciona; `cargo test` e benchmarks vao para o CI Linux. Desenvolvimento continua possivel, com feedback mais lento. | **Nao** |
| 3 — RHI e backend grafico | Arvore de dependencias muito maior (`wgpu`, `winit`) e executavel de janela que precisa rodar **local**, com GPU. O CI nao substitui. | **Sim** |
| 4 — Renderer | Criterio de aceite exige medir fps e alocacao por frame com GPU real. | **Sim** |
| 5 — Fisica | Visualizacao de debug depende do renderer. Testes de solver rodam no CI. | Parcial |
| 7 — Profiler e stress tests | Profiling de GPU e frame timing precisam da maquina real. | **Sim** |

### Decisao atual

**Tomada em 2026-09-03:** manter o SAC ligado. E uma protecao de seguranca cuja
desativacao e irreversivel, e a Fase 2 nao a justifica.

A Fase 2 avanca: ECS e job system nao precisam de GPU, o laco local de
`cargo check` funciona, e a verificacao vai para o CI Linux — ambiente
controlado e comparavel entre execucoes, em alguns aspectos melhor que a maquina
de desenvolvimento para detectar regressao.

Custo aceito, maior do que o estimado quando a decisao foi tomada: o feedback de
teste sai de segundos para o tempo de um push.

### Gatilho de reavaliacao

**Inicio da Fase 3.** Antes de tratar o desligamento do SAC como saida, testar
autorizacao administrada dos binarios de desenvolvimento: politica WDAC
suplementar confiando no diretorio de build, ou assinatura dos binarios com
certificado proprio confiado por politica. So se essas vias falharem a escolha
passa a ser entre sacrificar o fluxo local nativo e desativar o SAC.

Desativar o SAC e irreversivel sem reinstalar o Windows.

---

## R02 — Determinismo em ponto flutuante entre plataformas

**Status:** aberto — mitigacao planejada para a Fase 2
**Decisao relacionada:** [D09](DECISOES.md)

Determinismo bit-exato de `f32` entre Windows e Linux e fragil: diferencas em
funcoes transcendentais da libm, reordenacao do otimizador e reducoes paralelas
com ordem instavel produzem divergencia. Save/load, replay e networking em
lockstep dependem disso (secoes 15 e 16 do documento de visao).

**Mitigacao:** teste de determinismo cross-platform no CI desde a Fase 2, quando
consertar ainda e barato. Ordem de iteracao estavel ja e propriedade garantida e
testada do `Pool`.

**Gatilho de reavaliacao:** primeira divergencia observada entre Windows e Linux.
Se f32 se mostrar impraticavel, a alternativa e fixed-point na simulacao — caro,
e por isso o teste vem cedo.

---

## R03 — ECS proprio pode ficar atras das implementacoes existentes

**Status:** aberto — medicao planejada para a Fase 2
**Decisao relacionada:** [D02](DECISOES.md)

Escrever o ECS e a decisao mais cara de reverter do projeto. Se a implementacao
propria for consistentemente mais lenta que `bevy_ecs` ou `hecs`, o custo do
diferencial nao se paga.

**Mitigacao:** `bevy_ecs` e `hecs` entram como baseline nos benchmarks da Fase 2.
A comparacao e dado para decidir, nao para ignorar.

**Gatilho de reavaliacao:** fim da Fase 2, com numeros na mao.

---

## R04 — A RHI virar um clone da API do wgpu

**Status:** aberto — verificacao na Fase 8
**Decisao relacionada:** [D01](DECISOES.md)

Com um unico backend, nada impede que as traits da RHI absorvam o vocabulario e
as premissas do `wgpu`. A abstracao so estara provada quando existir um segundo
backend.

**Mitigacao:** desenhar cada trait olhando tambem para Vulkan e DX12, com nota de
mapeamento nos tipos nao obvios. Verificacao de isolamento ja roda no CI.

**Gatilho de reavaliacao:** backend Vulkan nativo da Fase 8. Cada concessao
necessaria ali e evidencia de vazamento.

---

## R05 — Escopo: uma engine e trabalho de equipe

**Status:** aberto — mitigacao estrutural no proprio plano

**Mitigacao:** fatia vertical nas Fases 1 a 4, entregando algo executavel cedo;
nenhuma feature entra sem um caso de uso que a exija; toda fase tem criterio de
aceite medido, o que impede declarar pronto o que nao esta.
