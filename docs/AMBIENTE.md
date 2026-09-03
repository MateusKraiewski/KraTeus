# Ambiente de Desenvolvimento

Estado verificado da maquina de desenvolvimento e o que precisa de decisao.

## Verificado em 2026-09-03

| Item | Estado |
|---|---|
| Rust | `rustc 1.98.1` / `cargo 1.98.1`, toolchain `stable-x86_64-pc-windows-msvc` |
| Linker MSVC | OK — VS Build Tools 2022 e 2026, VS Community 2026, Windows SDK presente |
| `rustfmt` | OK (`1.9.0-stable`) |
| `clippy` | **Bloqueado** — ver abaixo |
| Compilar e rodar binario proprio | OK, com a ressalva do Smart App Control |

## Smart App Control: o problema real desta maquina

`VerifiedAndReputablePolicyState = 1` (enforce) e
`UsermodeCodeIntegrityPolicyEnforcementStatus = 2` (enforced).

O Smart App Control impede a execucao de binarios e o carregamento de DLLs que
ele nao reconhece por reputacao. Como todo binario recem-compilado e, por
definicao, desconhecido, isso atinge o desenvolvimento em Rust em tres pontos:

1. **DLLs de proc-macro.** `serde`, `thiserror`, `tracing` e praticamente todo o
   ecossistema usam proc-macros, que compilam para `.dll` carregada pelo
   `rustc`. Bloqueio observado no log do Code Integrity, evento 3077:

   ```
   rustc.exe attempted to load thiserror_impl-fb279873d94226fc.dll
   that did not meet the Enterprise signing level requirements
   ```

2. **Build scripts.** `build-script-build.exe` bloqueado com `os error 4551`
   quando gerado fora do perfil do usuario.

3. **Binarios de teste e executaveis do projeto.** Este e o pior caso, e o
   comportamento observado e mais grave do que parecia a principio:

   - O binario de teste de `krateus-rhi-wgpu` foi bloqueado enquanto os outros
     dez, no mesmo diretorio, rodaram sem problema.
   - **O veredito muda com o tempo para o mesmo binario.** O binario
     `krateus_core-8536b4e19a308749.exe` executou os 19 testes com sucesso e,
     minutos depois, passou a ser bloqueado — mesmo hash, mesmo caminho. O SAC
     consulta reputacao na nuvem de forma assincrona e revoga o que ja havia
     liberado.
   - Forcar um hash novo (`-C metadata=...`) nao contorna.

   Consequencia pratica: **`cargo test` nao e confiavel nesta maquina**, nem
   para codigo puro de CPU. Nao e um sorteio por build; e um veredito que pode
   mudar entre duas execucoes do mesmo binario.

### Mitigacao parcial ja aplicada

`.cargo/config.toml` (nao versionado) redireciona o `target/` para dentro de
`%LOCALAPPDATA%`:

```toml
[build]
target-dir = "C:/Users/mateu/AppData/Local/krateus-target"
```

Isto resolve os pontos 1 e 2: com o `target/` em `E:\` — ou mesmo em `C:\` fora
do perfil do usuario — o carregamento de proc-macro falha de forma consistente.
Dentro do perfil, funciona.

**Nao resolve o ponto 3, nem sempre resolve o ponto 2.** Com o `target/` ja
dentro do perfil do usuario, o build script de `num-traits` — dependencia do
`criterion` — continua bloqueado, de forma reproduzivel. Ou seja: o harness de
benchmark da Fase 1 **nao compila nesta maquina** enquanto o SAC estiver ligado.

O plano se apoia em "medir com benchmarks" (secao 19 do documento de visao).
Sem `criterion`, os criterios de aceite de performance nao podem ser verificados
**localmente** — mas podem ser verificados no CI Linux, onde o SAC nao existe.
Ver a analise de impacto por fase em [RISCOS.md](RISCOS.md).

### Decisao tomada em 2026-09-03: manter o SAC ligado

O SAC permanece ativo. E uma protecao de seguranca cuja desativacao e
irreversivel, e a Fase 2 nao a justifica.

> **Desligar o Smart App Control e irreversivel.** O Windows nao permite
> religa-lo; voltar atras exige reinstalacao limpa do sistema.

**Fluxo adotado:**

| Comando | Local | Observacao |
|---|---|---|
| `cargo check --workspace --all-targets` | ✅ confiavel | Nao executa nada; e o laco de trabalho local |
| `cargo build` | ✅ confiavel | Compila; executar o resultado e que e incerto |
| `cargo fmt` | ✅ confiavel | |
| `cargo run --example` | ⚠️ intermitente | Funciona ate o SAC decidir o contrario |
| `cargo test` | ❌ nao confiavel | Ver ponto 3 acima |
| `cargo clippy` | ❌ bloqueado | |
| `cargo bench` | ❌ bloqueado | `criterion` → `zerocopy`, `num-traits` |

O laco local passa a ser **`cargo check` + `cargo fmt`**, com o CI Linux como
autoridade sobre testes, clippy, `cargo-deny` e benchmarks.

**`cargo check` em debug nao basta.** O job de benchmark compila em release, onde
`debug_assertions` esta desligado — codigo alcancado so por `cfg(debug_assertions)`
some, e o que existia para ele vira codigo morto sob `-D warnings`. Isso ja
reprovou um checkpoint. Antes de enviar, rodar tambem:

```powershell
cargo check --workspace --all-targets --all-features   # perfil dev
cargo check --workspace --release --all-targets        # perfil de benchmark
``` A cobertura nao e
perdida — o que se perde e a latencia do feedback, que sai de segundos para o
tempo de um push.

Isto e pior do que o previsto quando a decisao foi tomada: a expectativa era que
`cargo test` funcionasse localmente para codigo de CPU. Nao funciona. A decisao
de manter o SAC continua valendo, mas com esse custo maior registrado.

### Antes da Fase 3: testar autorizacao administrada

A Fase 3 traz GPU, `winit` e uma arvore de dependencias muito maior — e o
momento em que o atrito local deixa de ser contornavel. Antes de tratar o
desligamento do SAC como unica saida, verificar se os binarios de
desenvolvimento podem ser autorizados de forma administrada:

- Politica WDAC suplementar que confie no diretorio de build ou no signatario.
- Assinatura dos binarios de desenvolvimento com certificado proprio, e confianca
  nesse certificado via politica.
- Configuracao equivalente oferecida pela politica ja ativa na maquina
  (Policy ID `{0283ac0f-fff1-49ae-ada1-8a933130cad6}`), que pode ser gerenciada e
  nao apenas o SAC de consumidor.

Se nenhuma dessas vias funcionar, a escolha real passa a ser entre sacrificar o
fluxo local nativo — WSL2, VM ou dual boot, todas com custo para as Fases 3 a 5 —
e desativar o SAC conscientemente. Desativar e uma saida comum em maquina de
desenvolvimento pessoal, nao a unica nem uma necessidade tecnica inevitavel.

## `clippy` bloqueado

`cargo-clippy.exe` e `clippy-driver.exe` sao bloqueados pelo mesmo mecanismo, e
nao tem Mark-of-the-Web — ou seja, `Unblock-File` nao resolve, e nao ha excecao
por arquivo no SAC.

Enquanto isso nao se resolver:

- Localmente, usar `cargo check` no lugar de `cargo clippy`.
- O `clippy` continua rodando no CI, em Linux, onde o problema nao existe. A
  perda e de feedback local, nao de cobertura.

## Alternativas descartadas

Registradas para nao serem retentadas:

- **`Unblock-File`.** Os binarios bloqueados nao tem Mark-of-the-Web; o bloqueio
  e por reputacao, nao por zona de origem.
- **Mover o `target/` para `C:` fora do perfil.** `C:/krateus-target` e
  bloqueado exatamente como `E:`. Apenas `%LOCALAPPDATA%` funciona — e apenas
  em parte.
- **Reexecutar o build.** O bloqueio e estavel por hash: o mesmo binario falha
  em todas as tentativas. Nao adianta repetir.

## Comandos

O `cargo` nao esta no `PATH` das sessoes de shell; prefixar quando necessario:

```powershell
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"
cargo test --workspace
cargo run -p krateus-core --example hello_core
```
