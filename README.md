# vitrola-bt

[![CI](https://github.com/4snt/vitrola-bt/actions/workflows/ci.yml/badge.svg)](https://github.com/4snt/vitrola-bt/actions/workflows/ci.yml)

Ouça e grave no Linux o áudio Bluetooth de uma **vitrola** (testado com a Polyvox
XTD-67, que aparece como `LP3005_NP5`). O PC se passa por **caixa de som Bluetooth**,
a vitrola pareia nele e o disco toca nos alto-falantes do PC — ou vira um FLAC.

Testado em Pop!_OS 24.04 · PipeWire 1.6 · WirePlumber 0.5 · BlueZ 5.72.

> **Por que não é um "driver"?** O Linux já tem o driver Bluetooth (BlueZ) e o servidor
> de áudio (PipeWire), e a vitrola fala A2DP padrão. O que faltava era o PC **assumir o
> papel de receptor (A2DP Sink)**, **aparecer para a vitrola como caixa de som** e
> **aceitar o pareamento sozinho**. Este projeto faz isso.

## Instalar

**Pacote .deb** (Ubuntu/Pop!_OS/Debian): baixe o `.deb` da
[última release](https://github.com/4snt/vitrola-bt/releases/latest) e

```bash
sudo apt install ./vitrola-bt_*_all.deb
systemctl --user restart wireplumber pipewire pipewire-pulse
```

**A partir do código:**

```bash
git clone https://github.com/4snt/vitrola-bt && cd vitrola-bt
bin/vitrola instalar      # configs de áudio, comandos e ícone no menu (reinicia o áudio)
```

## Usar: app gráfico

Abra **Vitrola BT** no menu de aplicativos (ou `vitrola-gui`):

1. Ligue **Modo caixa de som** — na primeira vez o sistema pede sua senha para o disfarce.
2. Na vitrola, ligue o Bluetooth e escolha **"Vitrola PC"**.
3. Quando ela conectar, o som sai nos alto-falantes. **Gravar** salva FLAC em
   `~/Música/Vitrola` (ou `./gravacoes` no código clonado).

## Usar: terminal

```bash
vitrola receber           # PC vira "caixa de som" e a vitrola pareia NELE (recomendado)
vitrola parear            # alternativa: o PC procura a vitrola; ou: vitrola parear AA:BB:...
vitrola diagnostico       # confirma se ela TRANSMITE áudio e mostra o codec
vitrola ouvir             # toca o disco nas caixas/fone do PC
vitrola gravar lado-a     # grava em FLAC (lado-a.flac)
```

### `vitrola receber`: o PC se disfarça de caixa de som

O transmissor da vitrola procura **alto-falantes/fones**, não computadores. O PC se
anuncia como `Computer/Laptop` (classe `0x6c010c`), e a vitrola esconde isso da busca.
O `receber`:

1. muda a classe para **Audio/Video · Loudspeaker** (`sudo btmgmt class 4 20`; volta ao
   normal quando o Bluetooth reinicia);
2. fica visível como **"Vitrola PC"** (`VITROLA_ALIAS` muda o nome) e sem prazo para expirar;
3. roda o agente `vitrola_bt/agente.py`, que aceita o pareamento sem perguntar (Just Works ou PIN `0000`)
   e marca a vitrola como confiável;
4. quando chega o stream `bluez_input.*`, salva o MAC e toca nos alto-falantes; se cair,
   espera reconectar.

## Desenvolvimento

O CI (GitHub Actions) roda `shellcheck`, `ruff`, valida o `.desktop` e gera o `.deb` a
cada push/PR. Uma tag `vX.Y.Z` publica a release com o `.deb` anexado:

```bash
packaging/build-deb.sh 0.1.0      # gera dist/vitrola-bt_0.1.0_all.deb localmente
git tag v0.1.0 && git push --tags # release automática
```

## O que as configs fazem

| Arquivo | Efeito |
|---|---|
| `config/wireplumber/51-vitrola-bt.conf` | Liga papel **A2DP Sink** (PC recebe); anuncia codecs `aptX HD > aptX > AAC > SBC-XQ > SBC`; habilita **SBC-XQ**; impede troca automática para HFP (som de telefone); buffer maior e sem suspensão no nó da vitrola |
| `config/pipewire/51-vitrola-rates.conf` | Permite 44.1 e 48 kHz no grafo → sem reamostragem do stream |

São instaladas como **symlinks** em `~/.config/...` — editar aqui vale lá
(depois `systemctl --user restart wireplumber pipewire pipewire-pulse`).

### Codecs: o que esperar

Quem escolhe o codec é o **transmissor** (a vitrola). O PC anuncia tudo que sabe decodificar:

- **aptX HD** (24 bit) / **aptX** — se a vitrola tiver; decodificado via `libfreeaptx` (já instalado).
- **AAC** — só funciona se o PipeWire tiver sido compilado com `fdk-aac`; o pacote do Pop!_OS não traz
  `libspa-codec-bluez5-aac.so`, então é ignorado.
- **SBC-XQ** — SBC com bitpool alto; só é usado se o transmissor aceitar.
- **SBC** — o mais provável numa vitrola de entrada. Ainda assim fica ~328 kbps, bem acima de HFP.
- **LDAC** — não existe decodificador livre: o Linux **envia** LDAC, mas não **recebe**.

`vitrola status` mostra o codec realmente negociado.

## Minha vitrola transmite Bluetooth?

A **XTD-67 transmite** (testado): anuncia Audio Source e manda SBC a 48 kHz. Outras
vitrolas descritas com "receptor Bluetooth" podem só **receber** (tocar o celular nas
caixas delas). `vitrola diagnostico` confere pelos perfis anunciados
(UUID `0x110A` Audio Source = transmite; só `0x110B` Audio Sink = só recebe).

### Plano B (e a melhor qualidade de verdade): cabo

Mesmo que transmita, Bluetooth sempre comprime. Para digitalizar discos com qualidade máxima:

1. Saída **RCA (line out)** da vitrola → **interface de áudio USB** com entrada de linha
   (ex.: Behringer UCA202/UCA222, ~R$150). Evite a entrada de microfone do notebook (o
   `sof-hda-dsp` deste PC só tem mic digital/headset mono).
2. Grave em 24 bit / 96 kHz sem perdas:
   ```bash
   pw-record --target <nome-da-interface> --rate 96000 --format s24 --channels 2 disco.flac
   ```
   (`wpctl status` lista os nomes.)

## Problemas comuns

- **Som de telefone / mono** → caiu em HFP: `vitrola fixar-a2dp`.
- **Estalos/cortes** → aproxime a vitrola, afaste do roteador 2.4 GHz, ou aumente
  `node.latency` (ex.: `2048/48000`) no conf do WirePlumber.
- **Não reconecta sozinha** → `bluetoothctl trust <MAC>` (o `parear` já faz).
- **Adaptador desligado** → `bluetoothctl power on` (o `parear` já faz).

## Licença

MIT — veja [LICENSE](LICENSE).
