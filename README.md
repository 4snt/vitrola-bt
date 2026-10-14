# vitrola-bt

Captura no Linux (Pop!_OS · PipeWire 1.6 · WirePlumber 0.5 · BlueZ 5.72) do áudio
Bluetooth da vitrola **Polyvox XTD-67**, com o melhor codec possível.

> **Por que não é um "driver" de kernel?** O Linux já tem o driver Bluetooth (BlueZ)
> e o servidor de áudio (PipeWire). O que define a qualidade é **qual papel o PC
> assume (A2DP Sink)** e **qual codec é negociado**. Este projeto configura isso e
> traz uma ferramenta `vitrola` para parear, monitorar, ouvir e gravar.

## Uso rápido

```bash
cd ~/projetos/vitrola-bt
bin/vitrola instalar      # aplica configs e reinicia o áudio (feche chamadas antes)
vitrola parear            # vitrola em modo pareamento; ou: vitrola parear AA:BB:...
vitrola diagnostico       # confirma se ela TRANSMITE áudio e mostra o codec
vitrola ouvir             # toca o disco nas caixas/fone do PC
vitrola gravar lado-a     # grava em FLAC em ./gravacoes/lado-a.flac
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

## ⚠️ Atenção: a XTD-67 transmite ou só recebe Bluetooth?

Lojas descrevem a XTD-67 com "receptor Bluetooth embutido" — isso costuma significar que o BT
serve para **tocar o celular nas caixas da vitrola**, e não para **enviar o disco**.
`vitrola diagnostico` verifica isso de forma objetiva pelos perfis anunciados
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
