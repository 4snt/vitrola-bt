# Licenças de terceiros

O Vitrola BT é distribuído sob a licença MIT. Ele usa os componentes abaixo.

## Compilados no binário (crates Rust)

Todas as dependências são de licença permissiva, sem copyleft forte. Resumo gerado com
`cargo metadata` (alvo x86_64-unknown-linux-gnu):

| Licença | Pacotes |
|---|---|
| MIT ou Apache-2.0 (variações, incluindo Zlib, BSD, 0BSD, Unlicense, LLVM-exception) | ~205 |
| MIT | 50 |
| Unicode-3.0 | 18 |
| Apache-2.0 | 12 |
| ISC, CC0-1.0 | 2 |
| MPL-2.0 (copyleft por arquivo; código-fonte em crates.io) | 1 |
| Fontes embutidas pelo egui: Ubuntu Font Licence 1.0 e SIL OFL 1.1 (Hack) | 1 |

O texto completo de cada licença acompanha o código-fonte de cada pacote no
[crates.io](https://crates.io). Principais: egui/eframe, zbus, tokio, rustfft, serde,
libloading, png, dirs.

## Usados em tempo de execução (não incluídos no pacote)

| Componente | Licença | Como é usado |
|---|---|---|
| libsbc (BlueZ SBC) | LGPL-2.1-or-later | carregada dinamicamente da instalação do sistema |
| BlueZ | GPL-2.0 | serviço do sistema, acessado por D-Bus (sem ligação de código) |
| PipeWire (`pw-cat`, `wpctl`) | MIT | processos do sistema |
| btmgmt, polkit, util-linux (`script`) | GPL/LGPL | programas do sistema chamados pelo ajudante |

## Ferramentas de empacotamento

[nFPM](https://nfpm.goreleaser.com/) (MIT), usado só para gerar os pacotes.
