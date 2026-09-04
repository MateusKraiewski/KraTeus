# Registro de Riscos — KraTeus Engine

Riscos com impacto em cronograma ou arquitetura. Um risco so sai daqui quando
for eliminado ou aceito de forma explicita, com a decisao registrada.

---

## R01 — Smart App Control bloqueia binarios de desenvolvimento

**Status:** aberto — bloqueante para a Fase 3; vias de autorizacao administrada
testadas e descartadas em 2026-09-04
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

### Reavaliação de 2026-09-04, antes da Fase 3

Como previsto no gatilho, as vias de autorização administrada foram testadas
antes de introduzir `wgpu`. O resultado é negativo, e o registro abaixo existe
para que ninguém precise repetir os testes.

### Estado medido

51 bloqueios em 24 horas, sobre 8 binários distintos: DLLs de proc-macro
(`thiserror_impl`, `tracing_attributes`), build scripts, `cargo-clippy.exe`,
binários de teste do próprio projeto e `rust_out.exe` — o executável de doctest.
O harness de benchmark (`criterion` → `num-traits`) segue bloqueado de forma
reproduzível.

### Assinatura com certificado próprio — **não funciona**

Testado diretamente: um binário bloqueado foi copiado, assinado com certificado
de assinatura de código autoassinado e executado de novo. Continuou bloqueado.
O Windows reporta a assinatura como inválida — "cadeia terminou em certificado
raiz que não é confiável".

Colocar a raiz no repositório confiável da máquina tornaria a *assinatura*
válida, mas não resolve: o Smart App Control não decide por confiança local. Ele
exige assinatura da Microsoft, ou um certificado com reputação estabelecida no
Intelligent Security Graph, ou reputação do próprio arquivo. Um certificado
recém-criado não tem nenhuma das três, e reputação não se concede localmente.

### Política WDAC suplementar — **não disponível**

Duas barreiras, ambas verificadas:

- `CiTool -lp` retorna acesso negado sem elevação, então nem enumerar as
  políticas ativas é possível no fluxo normal de desenvolvimento.
- A política do SAC (`{0283AC0F-FFF1-49AE-ADA1-8A933130CAD6}`, 7 KB, assinada
  pela Microsoft) é uma política de consumidor, ligada e desligada por um
  interruptor. Uma suplementar só pode ser acrescentada se a base declarar a
  opção correspondente, e adicionar qualquer política exige elevação a cada
  build — o que não é um fluxo de trabalho.

### Conclusão

**As duas vias de autorização administrada estão fechadas.** A escolha real,
como o registro anterior antecipava, ficou entre:

1. **Desativar o Smart App Control.** Irreversível sem reinstalar o Windows.
2. **Sacrificar o fluxo local nativo** — WSL2, VM ou dual boot. Todas custam
   caro justamente nas Fases 3 a 5, que precisam de GPU real na máquina.
3. **Seguir com o fluxo atual** — `cargo check` e `cargo clippy` locais, CI Linux
   como autoridade. Funciona até a Fase 3, quando aparece um executável de
   janela que precisa rodar localmente com GPU para que qualquer coisa possa ser
   vista ou medida.

A opção 3 deixa de ser suficiente exatamente no critério de aceite da Fase 3:
"triângulo colorido na tela, com resize e alt-tab estáveis". Isso não se verifica
por CI.

**Decisão pendente do responsável pela máquina.** Nada foi alterado: o
certificado de teste foi removido e nenhuma política foi tocada.

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

