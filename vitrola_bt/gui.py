"""Vitrola BT — interface gráfica (GTK 3).

O PC se disfarça de caixa de som Bluetooth, a vitrola pareia nele e o disco
toca nos alto-falantes do PC (e pode ser gravado em FLAC).
"""
import json
import os
import signal
import socket
import subprocess
import sys
import time
from pathlib import Path

import dbus
import dbus.mainloop.glib
import gi

gi.require_version("Gtk", "3.0")
from gi.repository import Gio, GLib, Gtk  # noqa: E402

from vitrola_bt import agente, bandeja  # noqa: E402

APP_ID = "io.github._4snt.VitrolaBT"
ALIAS = os.environ.get("VITROLA_ALIAS", "Vitrola PC")
ROOT = Path(__file__).resolve().parent.parent
POLL_MS = 2000
PREFS = Path(GLib.get_user_config_dir()) / "vitrola-bt" / "gui.json"

# Classe Bluetooth: 0x04 = Audio/Video; 0x14 = Loudspeaker. O PC por padrão é
# Computer/Laptop, e o transmissor da vitrola esconde isso da busca.
CLASS_MAJOR_AUDIO = 0x04
CLASS_MINOR_SPEAKER = 0x14

QUALIDADE = {
    "aptx_hd": "excelente (aptX HD, 24 bits)",
    "aptx": "muito boa (aptX)",
    "aac": "muito boa (AAC)",
    "sbc_xq": "boa (SBC-XQ)",
    "sbc": "padrão (SBC) — limitada pela vitrola",
}


def pasta_gravacoes():
    if os.environ.get("VITROLA_REC_DIR"):
        return Path(os.environ["VITROLA_REC_DIR"])
    local = ROOT / "gravacoes"
    if local.is_dir() and os.access(local, os.W_OK):
        return local
    musica = GLib.get_user_special_dir(GLib.UserDirectory.DIRECTORY_MUSIC) or str(Path.home() / "Música")
    return Path(musica) / "Vitrola"


def ler_prefs():
    padrao = {"minimizar_bandeja": True, "tocar_auto": True}
    try:
        padrao.update(json.loads(PREFS.read_text()))
    except (OSError, ValueError):
        pass
    return padrao


def salvar_prefs(prefs):
    try:
        PREFS.parent.mkdir(parents=True, exist_ok=True)
        PREFS.write_text(json.dumps(prefs, indent=2))
    except OSError:
        pass


class Bluetooth:
    """Acesso ao adaptador BlueZ pelo D-Bus do sistema."""

    def __init__(self, bus):
        self.bus = bus
        self.path = None

    def _objetos(self):
        om = dbus.Interface(self.bus.get_object("org.bluez", "/"), "org.freedesktop.DBus.ObjectManager")
        return om.GetManagedObjects()

    def adaptador(self):
        """Propriedades do adaptador (dict) ou None se não houver Bluetooth."""
        try:
            objs = self._objetos()
        except dbus.DBusException:
            return None
        for path, ifaces in objs.items():
            if "org.bluez.Adapter1" in ifaces:
                self.path = path
                return ifaces["org.bluez.Adapter1"]
        return None

    def set(self, prop, valor):
        props = dbus.Interface(self.bus.get_object("org.bluez", self.path), "org.freedesktop.DBus.Properties")
        props.Set("org.bluez.Adapter1", prop, valor)

    def vitrolas_conectadas(self):
        """Aparelhos conectados que anunciam A2DP Source (transmitem áudio)."""
        try:
            objs = self._objetos()
        except dbus.DBusException:
            return []
        achados = []
        for ifaces in objs.values():
            dev = ifaces.get("org.bluez.Device1")
            if dev and dev.get("Connected") and any(str(u).startswith("0000110a") for u in dev.get("UUIDs", [])):
                achados.append((str(dev.get("Alias", "?")), str(dev.get("Address", ""))))
        return achados


def ler_pw_dump(pw_dump_json):
    try:
        return json.loads(pw_dump_json or "[]")
    except ValueError:
        return []


def stream_bluetooth(objs):
    """Primeiro nó PipeWire bluez_input.* (o áudio vindo da vitrola)."""
    for o in objs:
        if o.get("type") != "PipeWire:Interface:Node":
            continue
        props = (o.get("info") or {}).get("props") or {}
        if str(props.get("node.name", "")).startswith("bluez_input."):
            params = (o.get("info") or {}).get("params") or {}
            fmt = (params.get("Format") or params.get("EnumFormat") or [{}])[0]
            rate = fmt.get("rate", 48000)
            if isinstance(rate, dict):
                rate = rate.get("default", 48000)
            return {
                "id": o["id"],
                "name": props["node.name"],
                "descricao": props.get("node.description") or props.get("device.description") or props["node.name"],
                "mac": props.get("api.bluez5.address", ""),
                "codec": props.get("api.bluez5.codec", ""),
                "perfil": props.get("api.bluez5.profile", ""),
                "rate": int(rate),
            }
    return None


def ligacoes_para_saidas(objs, node_id):
    """IDs dos links do nó da vitrola até saídas de som (Audio/Sink).

    O nó bluez_input é um Stream/Output/Audio: o WirePlumber o liga sozinho no
    alto-falante padrão, como faria com um player. "Ouvir" = existir esse link.
    """
    sinks = {
        o["id"] for o in objs
        if o.get("type") == "PipeWire:Interface:Node"
        and str(((o.get("info") or {}).get("props") or {}).get("media.class", "")).startswith("Audio/Sink")
    }
    return [
        o["id"] for o in objs
        if o.get("type") == "PipeWire:Interface:Link"
        and (o.get("info") or {}).get("output-node-id") == node_id
        and (o.get("info") or {}).get("input-node-id") in sinks
    ]


def definir_mudo(node_id, mudo):
    """Mudo no próprio stream da vitrola. O WirePlumber lembra desse estado e o
    reaplica quando a vitrola reconecta, então "Sair" continua mudo com o app fechado."""
    subprocess.run(["wpctl", "set-mute", str(node_id), "1" if mudo else "0"], capture_output=True)


def saida_padrao():
    """node.name do alto-falante padrão (metadado default.audio.sink)."""
    out = subprocess.run(["pw-metadata", "0", "default.audio.sink"], capture_output=True, text=True).stdout
    i = out.find('"name":"')
    return out[i + 8:out.index('"', i + 8)] if i >= 0 else None


class Janela(Gtk.ApplicationWindow):
    def __init__(self, app):
        super().__init__(application=app, title="Vitrola BT")
        self.set_default_size(480, -1)
        self.set_icon_name("vitrola-bt")

        dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
        self.bus = dbus.SystemBus()
        self.bt = Bluetooth(self.bus)
        self.agente = None
        self.alias_original = None
        self.stream = None
        self.links_saida = []
        self._mudando_ouvir = False
        self.proc_gravar = None
        self.arquivo_gravacao = None
        self.inicio_gravacao = 0.0
        self._mudando_switch = False
        self._estado_bandeja = None
        self._encerrado = False
        self.prefs = ler_prefs()

        hb = Gtk.HeaderBar(show_close_button=True, title="Vitrola BT", subtitle="Bluetooth da vitrola no PC")
        btn_sair = Gtk.Button.new_from_icon_name("system-shutdown-symbolic", Gtk.IconSize.BUTTON)
        btn_sair.set_tooltip_text("Sair de vez: para o som e desliga o modo caixa de som")
        btn_sair.connect("clicked", lambda _b: self.sair())
        hb.pack_end(btn_sair)
        self.set_titlebar(hb)

        caixa = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=14, margin=18)
        self.add(caixa)

        # 1. Modo caixa de som
        caixa.pack_start(self._titulo("1. Deixe o PC visível como caixa de som"), False, False, 0)
        linha = Gtk.Box(spacing=12)
        texto = Gtk.Label(xalign=0, wrap=True)
        texto.set_markup(f"<b>Modo caixa de som</b>\nA vitrola vai encontrar o PC como <b>“{ALIAS}”</b>")
        linha.pack_start(texto, True, True, 0)
        self.switch = Gtk.Switch(valign=Gtk.Align.CENTER)
        self.switch.connect("notify::active", self._on_switch)
        linha.pack_end(self.switch, False, False, 0)
        caixa.pack_start(linha, False, False, 0)
        self.lbl_bt = Gtk.Label(xalign=0, wrap=True)
        self.lbl_bt.get_style_context().add_class("dim-label")
        caixa.pack_start(self.lbl_bt, False, False, 0)

        caixa.pack_start(Gtk.Separator(), False, False, 0)

        # 2. Vitrola
        caixa.pack_start(self._titulo("2. Na vitrola, ligue o Bluetooth e escolha o PC"), False, False, 0)
        self.lbl_vitrola = Gtk.Label(xalign=0, wrap=True)
        caixa.pack_start(self.lbl_vitrola, False, False, 0)
        self.lbl_codec = Gtk.Label(xalign=0, wrap=True, selectable=True)
        self.lbl_codec.get_style_context().add_class("dim-label")
        caixa.pack_start(self.lbl_codec, False, False, 0)

        botoes = Gtk.Box(spacing=8)
        self.btn_ouvir = Gtk.ToggleButton(label="▶  Ouvir")
        self.btn_ouvir.connect("toggled", self._on_ouvir)
        self.btn_gravar = Gtk.ToggleButton(label="●  Gravar")
        self.btn_gravar.connect("toggled", self._on_gravar)
        btn_pasta = Gtk.Button(label="Gravações")
        btn_pasta.connect("clicked", self._on_pasta)
        for b in (self.btn_ouvir, self.btn_gravar):
            botoes.pack_start(b, True, True, 0)
        botoes.pack_end(btn_pasta, False, False, 0)
        caixa.pack_start(botoes, False, False, 0)

        self.chk_auto = Gtk.CheckButton(label="Tocar automaticamente quando a vitrola conectar",
                                        active=self.prefs["tocar_auto"])
        self.chk_auto.connect("toggled", self._on_pref)
        caixa.pack_start(self.chk_auto, False, False, 0)
        self.chk_bandeja = Gtk.CheckButton(label="Ao fechar a janela, continuar na bandeja (sem parar o som)",
                                           active=self.prefs["minimizar_bandeja"])
        self.chk_bandeja.connect("toggled", self._on_pref)
        caixa.pack_start(self.chk_bandeja, False, False, 0)

        # Log
        exp = Gtk.Expander(label="Detalhes")
        self.log_buf = Gtk.TextBuffer()
        tv = Gtk.TextView(buffer=self.log_buf, editable=False, monospace=True, wrap_mode=Gtk.WrapMode.WORD_CHAR)
        sc = Gtk.ScrolledWindow(min_content_height=140)
        sc.add(tv)
        self.log_view = tv
        exp.add(sc)
        caixa.pack_start(exp, True, True, 0)

        self.connect("delete-event", self._on_fechar)
        self.bandeja = None
        try:
            sessao = dbus.SessionBus()
            if bandeja.disponivel(sessao):
                self.bandeja = bandeja.Bandeja(sessao, self._itens_bandeja, self._alternar_janela,
                                               self._on_menu_bandeja)
        except dbus.DBusException as e:
            print(f"bandeja indisponível: {e}", flush=True)
        if self.bandeja is None:
            self.chk_bandeja.set_sensitive(False)
            self.chk_bandeja.set_tooltip_text("Este desktop não tem bandeja do sistema (StatusNotifier)")
        self._atualizar()
        GLib.timeout_add(POLL_MS, self._tick)

    @staticmethod
    def _titulo(txt):
        lbl = Gtk.Label(xalign=0)
        lbl.set_markup(f"<b>{GLib.markup_escape_text(txt)}</b>")
        return lbl

    def log(self, msg):
        fim = self.log_buf.get_end_iter()
        self.log_buf.insert(fim, f"{time.strftime('%H:%M:%S')}  {msg}\n")
        self.log_view.scroll_to_iter(self.log_buf.get_end_iter(), 0, False, 0, 0)
        print(msg, flush=True)

    # ---------- modo caixa de som ----------

    def _on_switch(self, sw, _pspec):
        if self._mudando_switch:
            return
        if sw.get_active():
            self._ligar_modo()
        else:
            self._desligar_modo()

    def _set_switch(self, ativo):
        self._mudando_switch = True
        self.switch.set_active(ativo)
        self._mudando_switch = False

    def _ligar_modo(self):
        subprocess.run(["rfkill", "unblock", "bluetooth"], check=False, capture_output=True)
        ad = self.bt.adaptador()
        if ad is None:
            self.log("Nenhum adaptador Bluetooth encontrado.")
            self._set_switch(False)
            return
        if not ad.get("Powered"):
            try:
                self.bt.set("Powered", dbus.Boolean(True))
                self.log("Bluetooth ligado")
            except dbus.DBusException as e:
                self.log(f"Não consegui ligar o Bluetooth: {e.get_dbus_message()}")
                self._set_switch(False)
                return
        classe = int(ad.get("Class", 0))
        if (classe >> 8) & 0x1F != CLASS_MAJOR_AUDIO:
            self.log("Disfarçando o PC de caixa de som (pede sua senha)…")
            proc = Gio.Subprocess.new(
                ["pkexec", "btmgmt", "class", str(CLASS_MAJOR_AUDIO), str(CLASS_MINOR_SPEAKER)],
                Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_PIPE,
            )
            proc.wait_check_async(None, self._classe_pronta)
        else:
            self._anunciar()

    def _classe_pronta(self, proc, res):
        try:
            proc.wait_check_finish(res)
            self.log("PC agora se apresenta como caixa de som")
        except GLib.Error:
            self.log("Classe não alterada (senha cancelada?). Seguindo como computador; a vitrola pode não achar o PC.")
        self._anunciar()

    def _anunciar(self):
        try:
            ad = self.bt.adaptador()
            if self.alias_original is None:
                atual = str(ad.get("Alias", ""))
                # Se o disfarce sobrou de uma sessão anterior, o original é o nome da máquina
                self.alias_original = socket.gethostname() if atual in ("", ALIAS) else atual
            self.bt.set("Alias", dbus.String(ALIAS))
            self.bt.set("DiscoverableTimeout", dbus.UInt32(0))
            self.bt.set("Pairable", dbus.Boolean(True))
            self.bt.set("Discoverable", dbus.Boolean(True))
            if self.agente is None:
                self.agente = agente.registrar(self.bus, lambda m: GLib.idle_add(self.log, m))
            self.log(f"Visível como “{ALIAS}”. Procure por ele na vitrola.")
        except dbus.DBusException as e:
            self.log(f"Erro ao anunciar: {e.get_dbus_message()}")
            self._set_switch(False)
        self._atualizar()

    def _desligar_modo(self):
        try:
            if self.bt.adaptador() is not None:
                self.bt.set("Discoverable", dbus.Boolean(False))
                if self.alias_original:
                    self.bt.set("Alias", dbus.String(self.alias_original))
        except dbus.DBusException:
            pass
        if self.agente is not None:
            agente.desregistrar(self.bus, self.agente)
            self.agente = None
        self.log("Modo caixa de som desligado")
        self._atualizar()

    # ---------- monitoramento ----------

    def _tick(self):
        proc = Gio.Subprocess.new(["pw-dump"], Gio.SubprocessFlags.STDOUT_PIPE | Gio.SubprocessFlags.STDERR_SILENCE)
        proc.communicate_utf8_async(None, None, self._pw_dump_pronto)
        return True

    def _pw_dump_pronto(self, proc, res):
        try:
            _ok, out, _err = proc.communicate_utf8_finish(res)
        except GLib.Error:
            out = "[]"
        objs = ler_pw_dump(out)
        novo = stream_bluetooth(objs)
        antes = self.stream
        self.stream = novo
        self.links_saida = ligacoes_para_saidas(objs, novo["id"]) if novo else []
        if novo and not antes:
            self.log(f"Áudio chegando de {novo['descricao']} ({novo['codec'] or '?'}, {novo['rate']} Hz)")
            # O WirePlumber já liga o stream nos alto-falantes (e precisa: sem
            # consumidor a vitrola para de transmitir). Ajusta e tira o mudo do "Sair".
            if self.chk_auto.get_active() and not self.links_saida:
                self._ligar_saida()
            elif not self.chk_auto.get_active() and self.links_saida:
                self._desligar_saida()
            definir_mudo(novo["id"], False)
        elif antes and not novo:
            self.log("A vitrola parou de transmitir / desconectou")
            self.btn_gravar.set_active(False)
        self._atualizar()

    def _atualizar(self):
        ad = self.bt.adaptador()
        if ad is None:
            self.lbl_bt.set_text("Bluetooth indisponível (desligado no sistema ou sem adaptador).")
        else:
            classe = int(ad.get("Class", 0))
            disfarce = "caixa de som" if (classe >> 8) & 0x1F == CLASS_MAJOR_AUDIO else "computador"
            partes = [
                "Bluetooth ligado" if ad.get("Powered") else "Bluetooth desligado",
                f"aparece como {disfarce}",
                f"visível como “{ad.get('Alias')}”" if ad.get("Discoverable") else "invisível",
            ]
            self.lbl_bt.set_text(" · ".join(partes))

        s = self.stream
        if s:
            self.lbl_vitrola.set_markup(f"<big>🎵 <b>{GLib.markup_escape_text(s['descricao'])}</b> transmitindo</big>")
            qual = QUALIDADE.get(s["codec"], "perfil não-A2DP (som de telefone)")
            self.lbl_codec.set_text(f"Codec {s['codec'] or '?'} · {s['rate']} Hz · qualidade {qual}\n{s['mac']}")
        else:
            conectadas = self.bt.vitrolas_conectadas() if ad else []
            if conectadas:
                nome, mac = conectadas[0]
                self.lbl_vitrola.set_markup(f"<big>🔗 <b>{GLib.markup_escape_text(nome)}</b> conectada</big>")
                self.lbl_codec.set_text(f"Coloque o disco para tocar · {mac}")
            else:
                self.lbl_vitrola.set_markup("<big>⏳ Aguardando a vitrola…</big>")
                self.lbl_codec.set_text("Ligue o modo caixa de som acima e procure o PC na vitrola.")

        tocando = bool(s and self.links_saida)
        if tocando != self.btn_ouvir.get_active():
            self._mudando_ouvir = True
            self.btn_ouvir.set_active(tocando)
            self._mudando_ouvir = False
        self.btn_ouvir.set_label("■  Parar de ouvir" if tocando else "▶  Ouvir")
        self.btn_ouvir.set_sensitive(s is not None)
        self.btn_gravar.set_sensitive(s is not None or self.btn_gravar.get_active())
        self._atualizar_bandeja(tocando)
        if self.proc_gravar:
            seg = int(time.time() - self.inicio_gravacao)
            self.btn_gravar.set_label(f"■  Parar ({seg // 60:02d}:{seg % 60:02d})")

    # ---------- ouvir / gravar ----------

    def _on_ouvir(self, btn):
        if self._mudando_ouvir:
            return
        if not self.stream:
            self._mudando_ouvir = True
            btn.set_active(False)
            self._mudando_ouvir = False
        elif btn.get_active():
            self._ligar_saida()
        else:
            self._desligar_saida()
        self._atualizar()

    def _ligar_saida(self):
        sink = saida_padrao()
        if not sink:
            self.log("Não achei a saída de som padrão")
            return
        definir_mudo(self.stream["id"], False)
        r = subprocess.run(["pw-link", self.stream["name"], sink], capture_output=True, text=True)
        if r.returncode == 0 or "File exists" in r.stderr:
            self.links_saida = self.links_saida or [-1]
            self.log("Tocando nos alto-falantes")
        else:
            self.log(f"Não consegui ligar nos alto-falantes: {r.stderr.strip()}")

    def _desligar_saida(self):
        # Só desfaz os links até as saídas de som; a gravação (pw-record) segue
        for link_id in self.links_saida:
            if link_id >= 0:
                subprocess.run(["pw-link", "-d", str(link_id)], capture_output=True)
        self.links_saida = []
        self.log("Parou de tocar (a vitrola segue conectada)")

    def _on_gravar(self, btn):
        if btn.get_active() and self.stream and not self.proc_gravar:
            pasta = pasta_gravacoes()
            pasta.mkdir(parents=True, exist_ok=True)
            self.arquivo_gravacao = pasta / f"disco-{time.strftime('%Y%m%d-%H%M%S')}.flac"
            fmt = "s24" if self.stream["codec"] == "aptx_hd" else "s16"
            self.proc_gravar = subprocess.Popen([
                "pw-record", "--target", self.stream["name"], "--rate", str(self.stream["rate"]),
                "--channels", "2", "--format", fmt, str(self.arquivo_gravacao),
            ])
            self.inicio_gravacao = time.time()
            self.log(f"Gravando em {self.arquivo_gravacao}")
            self._atualizar()
        elif not btn.get_active() and self.proc_gravar:
            self._parar(self.proc_gravar)
            self.proc_gravar = None
            btn.set_label("●  Gravar")
            self.log(f"Gravação salva: {self.arquivo_gravacao}")
        elif btn.get_active() and not self.stream:
            btn.set_active(False)

    @staticmethod
    def _parar(proc):
        # SIGINT faz o pw-record fechar o FLAC corretamente
        proc.send_signal(signal.SIGINT)
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()

    def _on_pasta(self, _btn):
        pasta = pasta_gravacoes()
        pasta.mkdir(parents=True, exist_ok=True)
        Gio.AppInfo.launch_default_for_uri(pasta.as_uri(), None)

    # ---------- fechar / bandeja ----------

    def _on_pref(self, _chk):
        self.prefs["tocar_auto"] = self.chk_auto.get_active()
        self.prefs["minimizar_bandeja"] = self.chk_bandeja.get_active()
        salvar_prefs(self.prefs)

    def _on_fechar(self, *_):
        if self.bandeja is not None and self.chk_bandeja.get_active():
            self.hide()
            self.log("Janela minimizada para a bandeja (o som continua)")
            self._atualizar_bandeja(bool(self.stream and self.links_saida), forcar=True)
            return True
        self.encerrar()
        return False

    def encerrar(self):
        """Fecha de verdade: para gravação e som e desliga o modo caixa de som."""
        if self._encerrado:
            return
        self._encerrado = True
        if self.proc_gravar:
            self._parar(self.proc_gravar)
            self.proc_gravar = None
        if self.stream:
            if self.links_saida:
                self._desligar_saida()
            definir_mudo(self.stream["id"], True)
        if self.agente is not None or self.switch.get_active():
            self._desligar_modo()
        if self.bandeja is not None:
            self.bandeja.remover()
            self.bandeja = None

    def sair(self):
        self.encerrar()
        self.get_application().quit()

    def _alternar_janela(self):
        if self.get_visible():
            self.hide()
        else:
            self.show_all()
            self.present()
        self._atualizar_bandeja(bool(self.stream and self.links_saida), forcar=True)

    def _itens_bandeja(self):
        itens = [(1, "Esconder janela" if self.get_visible() else "Mostrar janela")]
        if self.stream:
            itens.append((2, "Parar de ouvir" if self.links_saida else "Ouvir"))
        itens += [(3, None), (4, "Sair (para o som)")]
        return itens

    def _on_menu_bandeja(self, item):
        if item == 1:
            self._alternar_janela()
        elif item == 2 and self.stream:
            self.btn_ouvir.set_active(not self.btn_ouvir.get_active())
        elif item == 4:
            self.sair()

    def _atualizar_bandeja(self, tocando, forcar=False):
        if self.bandeja is None:
            return
        estado = (tocando, bool(self.stream), self.get_visible())
        if estado == self._estado_bandeja and not forcar:
            return
        self._estado_bandeja = estado
        if tocando:
            dica = f"Tocando: {self.stream['descricao']}"
        elif self.stream:
            dica = f"{self.stream['descricao']} conectada (sem som)"
        else:
            dica = "Aguardando a vitrola"
        self.bandeja.atualizar(dica)


class App(Gtk.Application):
    def __init__(self):
        super().__init__(application_id=APP_ID, flags=Gio.ApplicationFlags.FLAGS_NONE)
        self.janela = None

    def do_activate(self):
        if self.janela is None:
            self.janela = Janela(self)
        self.janela.show_all()
        self.janela.present()


def main():
    app = App()
    def ao_sigint():
        if app.janela:
            app.janela.encerrar()
        app.quit()
        return GLib.SOURCE_REMOVE

    GLib.unix_signal_add(GLib.PRIORITY_DEFAULT, signal.SIGINT, ao_sigint)
    return app.run(sys.argv)


if __name__ == "__main__":
    sys.exit(main())
