# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

Barra fina, sempre no topo, que mostra o uso da janela de 5 horas do Claude (conta do Claude Code). App Rust só para Windows (eframe/egui + Win32 via crate `windows`). README.md, código e comentários estão em português; mantenha o idioma.

## Comandos

```powershell
cargo test                      # todos os testes (unitários, em `mod tests` de cada arquivo)
cargo test <nome>               # um teste (ex.: cargo test width_fix); `cargo test auth::` roda um módulo
cargo run --release             # roda o app; exe em target\release\usage-bar-rs.exe
```

Não há config de lint/fmt própria: use `cargo clippy` e `cargo fmt` padrão. Os testes podem apontar para outro arquivo de credenciais com a env `USAGE_BAR_CRED`.

## Arquitetura

Duas threads que só conversam por canais `mpsc`:

- **UI (`app.rs`, `main.rs`)**: egui/eframe, janela sem borda, always-on-top, arrastar/redimensionar e menu de contexto. Nunca faz rede.
- **Worker (`worker.rs`)**: dono do login (`auth::Auth`) e de toda a rede. Recebe `Cmd` (`Refresh`, `UseLiveAccount`) e envia `Snapshot` (status, uso, aviso, conta). Consulta `api.anthropic.com/api/oauth/usage` a cada 180 s e no horário do reset; só `five_hour` é usado (`usage.rs`).

Módulos de apoio:

- `auth.rs`: login OAuth próprio do app, copiado de `~/.claude/.credentials.json` e guardado em `%LOCALAPPDATA%\UsageBarRs\login.bin` (DPAPI). Renovar rotaciona o refresh token, então antes de renovar o app adota um token mais novo que o Claude Code já tenha obtido e, depois, grava os tokens novos no arquivo do Claude Code se ele ainda tiver o refresh token antigo (preservando as outras chaves, sem BOM). Renovação forçada pós-401 no máximo a cada 30 min. Mexer aqui pode matar o login do Claude Code do usuário: leia o doc do módulo antes.
- `store.rs`: `config.json`, `login.bin`, `app.log`, escrita atômica (`write_atomic`).
- `win.rs`: tudo que é Win32 (mutex de instância única, janela tool/topmost, menu popup, autostart no registro, DPAPI `protect`/`unprotect`).
- `geom.rs`: largura em pontos lógicos, independente de DPI (guardar pixels físicos fazia a janela crescer 25% a cada abertura em monitor 125%).
- `alert.rs`: avisos sonoros. `App::drain` compara o nível da leitura nova com o anterior (`on_reading`) e toca só quando sobe; os WAVs são sintetizados em código e tocados da memória por `win::play_wav` (`PlaySoundW`). `cargo test plays_both_sounds -- --ignored` toca os dois para conferir de ouvido.
- `fmt.rs`: porcentagem, tempo restante e cores por nível (<50% azul, 50–84% âmbar, ≥85% vermelho).

## Commits

Conventional Commits, sempre em inglês (ex.: `feat(worker): add background worker`, `fix(auth): ...`, `docs: ...`, `chore: ...`).

## Cuidados

- Tokens nunca vão para o log: só o `auth::fingerprint` (SHA-256 truncado).
- `reqwest` usa `native-tls` (Schannel) de propósito; `serde_json` com `preserve_order` para reescrever o arquivo do Claude Code sem reordenar chaves.
