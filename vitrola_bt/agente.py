"""Agente de pareamento BlueZ que aceita tudo sem perguntar.

A vitrola (transmissor) não tem tela nem teclado: ela só pareia com "caixas"
que aceitam sozinhas (Just Works) ou com PIN fixo 0000. Este agente faz o PC
se comportar assim e marca o aparelho como confiável para reconectar sozinho.

Uso isolado: python3 -m vitrola_bt.agente
"""
import sys

import dbus
import dbus.mainloop.glib
import dbus.service

BUS = "org.bluez"
AGENT_PATH = "/vitrola/agente"
PIN = "0000"


def _log_stdout(msg):
    print(f"[agente] {msg}", flush=True)


class Agente(dbus.service.Object):
    def __init__(self, bus, log=_log_stdout):
        super().__init__(bus, AGENT_PATH)
        self.bus = bus
        self.log = log

    def _trust(self, device):
        try:
            props = dbus.Interface(self.bus.get_object(BUS, device), "org.freedesktop.DBus.Properties")
            props.Set("org.bluez.Device1", "Trusted", dbus.Boolean(True))
            name = props.Get("org.bluez.Device1", "Alias")
            self.log(f"aceito e confiável: {name}")
        except dbus.DBusException as e:
            self.log(f"não consegui marcar {device} como confiável: {e}")

    @dbus.service.method("org.bluez.Agent1", in_signature="", out_signature="")
    def Release(self):
        self.log("agente liberado pelo BlueZ")

    @dbus.service.method("org.bluez.Agent1", in_signature="os", out_signature="")
    def AuthorizeService(self, device, uuid):
        self.log(f"serviço {uuid[:8]} autorizado")
        self._trust(device)

    @dbus.service.method("org.bluez.Agent1", in_signature="o", out_signature="s")
    def RequestPinCode(self, device):
        self.log(f"PIN pedido -> {PIN}")
        self._trust(device)
        return PIN

    @dbus.service.method("org.bluez.Agent1", in_signature="o", out_signature="u")
    def RequestPasskey(self, device):
        self.log(f"passkey pedida -> {PIN}")
        self._trust(device)
        return dbus.UInt32(int(PIN))

    @dbus.service.method("org.bluez.Agent1", in_signature="ouq", out_signature="")
    def DisplayPasskey(self, device, passkey, entered):
        pass

    @dbus.service.method("org.bluez.Agent1", in_signature="os", out_signature="")
    def DisplayPinCode(self, device, pincode):
        self.log(f"PIN exibido: {pincode}")

    @dbus.service.method("org.bluez.Agent1", in_signature="ou", out_signature="")
    def RequestConfirmation(self, device, passkey):
        self.log(f"confirmação {passkey:06d} aceita")
        self._trust(device)

    @dbus.service.method("org.bluez.Agent1", in_signature="o", out_signature="")
    def RequestAuthorization(self, device):
        self.log("pareamento autorizado")
        self._trust(device)

    @dbus.service.method("org.bluez.Agent1", in_signature="", out_signature="")
    def Cancel(self):
        self.log("pareamento cancelado pelo outro lado")


def registrar(bus, log=_log_stdout):
    """Exporta o agente e o torna padrão. Devolve o objeto (guarde a referência)."""
    agente = Agente(bus, log)
    manager = dbus.Interface(bus.get_object(BUS, "/org/bluez"), "org.bluez.AgentManager1")
    manager.RegisterAgent(AGENT_PATH, "NoInputNoOutput")
    manager.RequestDefaultAgent(AGENT_PATH)
    return agente


def desregistrar(bus, agente):
    try:
        manager = dbus.Interface(bus.get_object(BUS, "/org/bluez"), "org.bluez.AgentManager1")
        manager.UnregisterAgent(AGENT_PATH)
    except dbus.DBusException:
        pass
    agente.remove_from_connection()


def main():
    from gi.repository import GLib

    dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
    bus = dbus.SystemBus()
    agente = registrar(bus)
    _log_stdout(f"pronto (NoInputNoOutput, PIN {PIN})")
    try:
        GLib.MainLoop().run()
    except KeyboardInterrupt:
        pass
    finally:
        desregistrar(bus, agente)
    return 0


if __name__ == "__main__":
    sys.exit(main())
