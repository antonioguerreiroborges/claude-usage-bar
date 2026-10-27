# usage-bar-rs

Barra fina, sempre no topo, que mostra o uso da **janela de 5 horas** do Claude para a conta logada no Claude Code. Mostra a porcentagem e o tempo até o reset, com cores por nível:

| Uso | Cor |
|---|---|
| até 49% | azul |
| 50% a 84% | âmbar |
| 85% ou mais | vermelho |

É uma versão simplificada, em Rust, do `claude-usage-widget-old`, que fica intocado na pasta ao lado.

## Requisitos

- Windows 10 ou 11, x64.
- Rust stable com toolchain MSVC (`winget install Rustlang.Rustup`) e o VS Build Tools ("Desktop development with C++").
- Claude Code logado ao menos uma vez (`claude`). Isso cria o `~/.claude/.credentials.json`.

## Compilar e rodar

```powershell
cargo test
cargo run --release
# exe portátil: target\release\usage-bar-rs.exe
```

## Uso

- **Arrastar** a barra move a janela.
- **Arrastar a borda direita** muda a largura. A partir de cerca de 230 px aparece também o horário do reset.
- **Botão direito** abre o menu:
  - conta, plano, horário do reset e da última atualização;
  - Atualizar agora;
  - Usar a conta atual do Claude Code;
  - Iniciar com o Windows;
  - Sair.
- A posição e a largura são lembradas.

## Como a conta é lida

1. Na primeira execução, o app copia o login OAuth do Claude Code e confirma email, org e plano no servidor. A cópia fica em `%LOCALAPPDATA%\UsageBarRs\login.bin`, criptografada com DPAPI (só o seu usuário do Windows consegue ler).
2. A barra continua seguindo essa conta mesmo se o Claude Code trocar de conta ou sair. Para seguir a conta atual do Claude Code, use **Usar a conta atual do Claude Code**.
3. O uso é lido a cada 180 s de `api.anthropic.com/api/oauth/usage`, e na hora do reset também. Só `five_hour` é usado.
4. Quando o access token está para expirar:
   - Se o Claude Code já renovou o mesmo login, o app adota esse token.
   - Senão, renova via `platform.claude.com/v1/oauth/token`. Se o arquivo do Claude Code ainda tiver o refresh token antigo, o app grava nele os tokens novos, preservando as outras chaves e sem BOM, para não deixar o Claude Code com um refresh token morto.

## Arquivos

Ficam em `%LOCALAPPDATA%\UsageBarRs\`:

| Arquivo | Conteúdo |
|---|---|
| `config.json` | posição, largura, conta (sem segredos), último uso lido |
| `login.bin` | cópia do login, criptografada com DPAPI |
| `app.log` | log; tokens só aparecem como fingerprint SHA-256 |

Os testes podem apontar para outro arquivo de credenciais com a variável `USAGE_BAR_CRED`.
