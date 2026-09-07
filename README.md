# KraTeus Engine

[![CI](https://github.com/MateusKraiewski/KraTeus/actions/workflows/ci.yml/badge.svg)](https://github.com/MateusKraiewski/KraTeus/actions/workflows/ci.yml)

Uma engine de jogos e simulações escrita em Rust, construída do zero — com a
regra de que **o custo da engine acompanha o que o projeto realmente usa**.

Não é uma engine pronta. É uma base sendo construída camada por camada, onde
cada uma só entra quando existe um caso de uso que a exija e um teste que a
verifique.

---

## Estado atual

**Desenvolvimento inicial.** O núcleo de simulação está completo e verificado;
a camada gráfica está começando.

| Fase | Escopo | Estado |
|---|---|---|
| **1** | Workspace, core, logging, config, CI | ✅ concluída |
| **2** | ECS, jobs, scheduler, loop de simulação | ✅ concluída |
| **3** | RHI e primeiro backend gráfico | 🚧 vocabulário e backend nulo prontos; **backend real não iniciado** |
| **5** | Física | 🚧 integrador e trajetória prontos; colisão não iniciada |
| 4, 6–8 | Renderer, mundo, editor, profiler, networking | ⬜ não iniciadas |

A Fase 5 foi começada fora de ordem por ser a única que avança sem placa de
vídeo. O que existe dela está listado abaixo; o que não existe está marcado como
não existindo.

A suíte roda na matriz Windows + Linux e inclui um teste de determinismo
cross-platform que compara o hash de estado após 300 passos de simulação.

### Por que o backend gráfico ainda não começou

O ambiente de desenvolvimento tem o **Smart App Control** do Windows em modo
enforce, que bloqueia a execução de binários recém-compilados por não terem
reputação. Isso impede o critério de aceite da Fase 3 — desenhar um triângulo
numa janela — que exige rodar um executável com acesso à GPU local.

Desde 7 de setembro de 2026 o bloqueio alcança o próprio `rustc`, e não há mais
compilação local de nenhuma espécie. O CI é hoje o único compilador do projeto.

As vias de autorização administrada foram testadas e descartadas; o registro
completo está em [`docs/RISCOS.md`](docs/RISCOS.md). O trabalho que não depende
de GPU foi feito e validado; o restante aguarda decisão sobre o ambiente.

---

## Princípios

- **Modularidade** — subsistemas independentes; um jogo 2D não carrega o que só
  o 3D usa.
- **Custo proporcional** — nada é pago antes de ser usado.
- **Dados separados da lógica** — ECS com layout SoA, favorável a cache.
- **Determinismo** — timestep fixo e ordem de iteração estável, verificados por
  teste entre plataformas.
- **Runtime separado do editor** — o editor é ferramenta, não parte do jogo.
- **Renderer desacoplado da API gráfica** — uma RHI entre o renderer e a GPU,
  com regra de CI que impede vazamento.

---

## Arquitetura

```
                    ┌──────────────────────────────┐
                    │        krateus-core          │
                    │  Handle · Pool · Clock       │
                    │  Config · Logging · JobPool  │
                    └──────────────┬───────────────┘
                                   │
         ┌─────────────────────────┼─────────────────────────┐
         │                         │                         │
┌────────▼────────┐      ┌─────────▼────────┐      ┌─────────▼────────┐
│   krateus-ecs   │      │ krateus-simula-  │      │   krateus-rhi    │
│                 │      │      tion        │      │                  │
│ archetypes      │◀─────│ timestep fixo    │      │ traits do        │
│ queries         │      │ determinismo     │      │ backend gráfico  │
│ scheduler       │      └──────────────────┘      │ + NullRhi        │
└────────┬────────┘                                └─────────▲────────┘
         │                                                   │
         │              ┌──────────────────┐                 │
         └─────────────▶│  krateus-render  │─────────────────┘
                        │   Render World   │
                        └──────────────────┘
```

### O caminho de um quadro

Os quatro estágios entre o mundo lógico e a GPU. O snapshot é **fechado**: não
referencia o `World`, então a simulação pode avançar enquanto o quadro anterior
ainda é desenhado.

```
  World  ──extract──▶  RenderSnapshot  ──prepare──▶  PreparedFrame
   ECS                  (imutável)                    (+ culling)
                                                            │
                                                          queue
                                                            ▼
   GPU  ◀── backend ◀──  RHI  ◀────render────────  RenderWork
                                                  (lotes canônicos)
```

| Estágio | Decide |
|---|---|
| `extract` | o que existe |
| `prepare` | o que aparece (frustum culling) |
| `queue` | como agrupar (lotes por material e malha) |
| `render` | nada — apenas traduz para a RHI |

---

## O que está implementado

### `krateus-core`
`Handle<T>` com geração, `Pool<T>` com slots reciclados, `Clock` de timestep
fixo com proteção contra espiral de morte, configuração em camadas
(TOML + variável de ambiente), logging estruturado por subsistema, e `JobPool`
— pool de threads com escopo que empresta da pilha.

### `krateus-ecs`
ECS próprio, baseado em archetypes:

- Armazenamento SoA com colunas type-erased
- Queries tipadas — `Query<(&mut Posicao, &Velocidade)>` — com filtros `With`,
  `Without`, `Changed` e `Added`
- Recursos globais em espaço de identificadores próprio
- Command buffer para mudanças estruturais diferidas
- **Scheduler híbrido**: etapas semânticas declaradas, paralelismo calculado
  automaticamente dentro de cada uma a partir dos tipos dos parâmetros
- Sistemas exclusivos que formam fronteira de etapa
- Change detection com ticks e comparação circular

### `krateus-simulation`
Laço de timestep fixo que publica `Time` como recurso, expõe `alpha` para
interpolação, e traz o teste de determinismo cross-platform.

### `krateus-rhi`
> ⚠️ **Interna e experimental.** Não é contrato público até existir um backend
> real. Ver [`docs/DECISOES.md`](docs/DECISOES.md), D12.

Vocabulário mínimo modelado a partir de Vulkan e DX12 — comandos gravados num
encoder e submetidos, não executados na chamada. Acompanha o **`NullRhi`**, que
não desenha nada e valida tudo: ordem de gravação, ciclo de vida de recursos,
compatibilidade de formatos. São 25 testes de contrato que rodam sem GPU e que o
backend real precisará passar.

### `krateus-render`
Render World completo, testado do mundo lógico ao comando de GPU contra o
`NullRhi`.

### `krateus-physics`
Início da Fase 5, feito enquanto a camada gráfica aguarda ambiente:

- **Integrador semi-implícito** com gravidade, forças e impulsos. A massa é
  guardada invertida, o que faz de "corpo estático" o valor `0.0` em vez de um
  caso especial. Dois testes lado a lado mostram por que o semi-implícito foi
  escolhido: ele mantém a energia de um oscilador dentro de 5% ao longo de mil
  passos, e o Euler explícito, no mesmo sistema, a infla.
- **Ferramenta de trajetória** do §6: dados altura do ápice, distância e
  desnível, devolve velocidades, tempos de subida, descida e voo. A solução usa
  apenas raiz quadrada e aritmética — sem `tan` nem `atan` —, porque funções
  transcendentais da libm são onde plataformas divergem.
- **Níveis de fidelidade** — analítica, integrada, integrada com arrasto — para
  que uma previsão de mira barata e uma granada com arrasto usem a mesma API com
  custos diferentes.

Ainda **não existem**: formas de colisão, broadphase, narrowphase, resolução de
contato, raycast, character controller e gizmos de debug.

---

## Roadmap

O plano completo, com critérios de aceite mensuráveis, está em
[`docs/ROADMAP.md`](docs/ROADMAP.md).

**Próximo marco — fechar a Fase 3:**

1. Implementar `krateus-rhi-wgpu`, com todo o `wgpu` isolado nessa crate
2. Criar janela, surface, adapter e device
3. Renderizar um triângulo colorido
4. Validar resize, alt-tab e apresentação contínua
5. Rodar os testes de contrato do `NullRhi` contra o backend real

Depois disso, a Fase 4 integra o Render World — que já está pronto — ao backend
real.

---

## Compilando e testando

Requer Rust **1.85** ou superior (edition 2024).

```bash
git clone https://github.com/MateusKraiewski/KraTeus
cd KraTeus

cargo build --workspace
cargo test --workspace
```

Exemplo executável — inicializa logging, lê configuração e roda 10.000 passos de
timestep fixo:

```bash
cargo run -p krateus-core --example hello_core
```

Benchmarks ficam atrás de uma feature, para que `criterion` não entre na árvore
de dependências do build normal:

```bash
cargo bench -p krateus-ecs --bench ecs --features bench
```

Antes de enviar uma mudança:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features
cargo check --workspace --release --all-targets --all-features
```

O último não é redundante: o perfil de release desliga `debug_assertions`, e
código alcançado apenas por ele vira código morto sob `-D warnings`.

---

## Contribuindo

Projeto pessoal em fase inicial, mas issues e pull requests são bem-vindos.

**Antes de abrir um PR:**

- O CI precisa passar nos cinco jobs: `fmt + clippy`, `cargo-deny`, testes em
  Windows e Linux, benchmarks e regras de arquitetura.
- Mudanças de arquitetura devem ser discutidas em uma issue antes do código.
  Decisões técnicas ficam registradas em [`docs/DECISOES.md`](docs/DECISOES.md),
  com justificativa e custo de reversão.
- Código novo vem com teste. Blocos `unsafe` vêm com comentário `// SAFETY:`
  explicando o argumento que os sustenta.

**Regras verificadas automaticamente pelo CI**, não apenas documentadas:

- Nenhuma referência a API gráfica fora da crate de backend
- `krateus-core` é folha no grafo de dependências
- `krateus-render` não depende de `krateus-simulation`

Comentários e documentação estão em português; nomes de tipos e funções seguem
convenção Rust.

---

## Licença

MIT ou Apache-2.0, à escolha de quem usar.
