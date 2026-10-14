"""Ícone na bandeja do sistema (StatusNotifierItem + menu com.canonical.dbusmenu).

Implementado direto no D-Bus para não depender de libayatana-appindicator:
funciona no COSMIC, KDE, XFCE, Budgie e no GNOME com a extensão AppIndicator.
"""
import os

import dbus
import dbus.service

SNI_PATH = "/StatusNotifierItem"
MENU_PATH = "/MenuBar"
WATCHER = "org.kde.StatusNotifierWatcher"
IFACE_SNI = "org.kde.StatusNotifierItem"
IFACE_MENU = "com.canonical.dbusmenu"
IFACE_PROPS = "org.freedesktop.DBus.Properties"


def disponivel(bus):
    try:
        return bool(bus.name_has_owner(WATCHER))
    except dbus.DBusException:
        return False


class _Item(dbus.service.Object):
    def __init__(self, bus, bandeja):
        super().__init__(bus, SNI_PATH)
        self.b = bandeja

    def _props(self):
        return {
            "Category": dbus.String("Hardware"),
            "Id": dbus.String("vitrola-bt"),
            "Title": dbus.String("Vitrola BT"),
            "Status": dbus.String("Active"),
            "IconName": dbus.String(self.b.icone),
            "IconThemePath": dbus.String(self.b.icon_theme_path),
            "ToolTip": dbus.Struct(
                (dbus.String(""), dbus.Array([], signature="(iiay)"), dbus.String("Vitrola BT"),
                 dbus.String(self.b.dica)),
                signature="sa(iiay)ss",
            ),
            "ItemIsMenu": dbus.Boolean(False),
            "Menu": dbus.ObjectPath(MENU_PATH),
            "WindowId": dbus.Int32(0),
        }

    @dbus.service.method(IFACE_PROPS, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return self._props()[prop]

    @dbus.service.method(IFACE_PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return self._props() if iface == IFACE_SNI else {}

    @dbus.service.method(IFACE_SNI, in_signature="ii", out_signature="")
    def Activate(self, x, y):
        self.b.ao_clicar()

    @dbus.service.method(IFACE_SNI, in_signature="ii", out_signature="")
    def SecondaryActivate(self, x, y):
        self.b.ao_clicar()

    @dbus.service.method(IFACE_SNI, in_signature="ii", out_signature="")
    def ContextMenu(self, x, y):
        pass

    @dbus.service.method(IFACE_SNI, in_signature="is", out_signature="")
    def Scroll(self, delta, orientation):
        pass

    @dbus.service.signal(IFACE_SNI, signature="")
    def NewToolTip(self):
        pass

    @dbus.service.signal(IFACE_SNI, signature="")
    def NewIcon(self):
        pass


class _Menu(dbus.service.Object):
    def __init__(self, bus, bandeja):
        super().__init__(bus, MENU_PATH)
        self.b = bandeja
        self.revisao = 1

    def _item(self, id_, props):
        return dbus.Struct((dbus.Int32(id_), dbus.Dictionary(props, signature="sv"),
                            dbus.Array([], signature="v")), signature="ia{sv}av", variant_level=1)

    def _filhos(self):
        filhos = []
        for id_, rotulo in self.b.itens():
            if rotulo is None:
                filhos.append(self._item(id_, {"type": dbus.String("separator")}))
            else:
                filhos.append(self._item(id_, {"label": dbus.String(rotulo), "enabled": dbus.Boolean(True),
                                               "visible": dbus.Boolean(True)}))
        return filhos

    def atualizar(self):
        self.revisao += 1
        self.LayoutUpdated(dbus.UInt32(self.revisao), dbus.Int32(0))

    @dbus.service.method(IFACE_MENU, in_signature="iias", out_signature="u(ia{sv}av)")
    def GetLayout(self, parent, depth, names):
        raiz = dbus.Struct((dbus.Int32(0), dbus.Dictionary({"children-display": dbus.String("submenu")},
                                                           signature="sv"),
                            dbus.Array(self._filhos(), signature="v")), signature="ia{sv}av")
        return dbus.UInt32(self.revisao), raiz

    @dbus.service.method(IFACE_MENU, in_signature="aias", out_signature="a(ia{sv})")
    def GetGroupProperties(self, ids, names):
        todos = {int(f[0]): f[1] for f in self._filhos()}
        return dbus.Array([dbus.Struct((dbus.Int32(i), todos.get(int(i), {})), signature="ia{sv}")
                           for i in ids], signature="(ia{sv})")

    @dbus.service.method(IFACE_MENU, in_signature="is", out_signature="v")
    def GetProperty(self, id_, name):
        for f in self._filhos():
            if int(f[0]) == id_ and name in f[1]:
                return f[1][name]
        return dbus.String("")

    @dbus.service.method(IFACE_MENU, in_signature="isvu", out_signature="")
    def Event(self, id_, event, data, timestamp):
        if event == "clicked":
            self.b.ao_escolher(int(id_))

    @dbus.service.method(IFACE_MENU, in_signature="a(isvu)", out_signature="ai")
    def EventGroup(self, eventos):
        for id_, event, data, ts in eventos:
            self.Event(id_, event, data, ts)
        return dbus.Array([], signature="i")

    @dbus.service.method(IFACE_MENU, in_signature="i", out_signature="b")
    def AboutToShow(self, id_):
        return False

    @dbus.service.method(IFACE_MENU, in_signature="ai", out_signature="aiai")
    def AboutToShowGroup(self, ids):
        return dbus.Array([], signature="i"), dbus.Array([], signature="i")

    @dbus.service.method(IFACE_PROPS, in_signature="ss", out_signature="v")
    def Get(self, iface, prop):
        return self.GetAll(iface)[prop]

    @dbus.service.method(IFACE_PROPS, in_signature="s", out_signature="a{sv}")
    def GetAll(self, iface):
        return {"Version": dbus.UInt32(3), "TextDirection": dbus.String("ltr"),
                "Status": dbus.String("normal"), "IconThemePath": dbus.Array([], signature="s")}

    @dbus.service.signal(IFACE_MENU, signature="ui")
    def LayoutUpdated(self, revision, parent):
        pass


class Bandeja:
    """itens: função que devolve [(id, rótulo | None p/ separador)].
    ao_clicar(): clique no ícone. ao_escolher(id): item do menu escolhido."""

    def __init__(self, bus, itens, ao_clicar, ao_escolher, icone="vitrola-bt", icon_theme_path=""):
        self.bus = bus
        self.itens = itens
        self.ao_clicar = ao_clicar
        self.ao_escolher = ao_escolher
        self.icone = icone
        self.icon_theme_path = icon_theme_path
        self.dica = ""
        self.nome = f"org.kde.StatusNotifierItem-{os.getpid()}-1"
        self._bus_name = dbus.service.BusName(self.nome, bus)
        self._item = _Item(bus, self)
        self._menu = _Menu(bus, self)
        watcher = bus.get_object(WATCHER, "/StatusNotifierWatcher")
        watcher.RegisterStatusNotifierItem(self.nome, dbus_interface=WATCHER)

    def atualizar(self, dica=None):
        if dica is not None and dica != self.dica:
            self.dica = dica
            self._item.NewToolTip()
        self._menu.atualizar()

    def remover(self):
        self._item.remove_from_connection()
        self._menu.remove_from_connection()
        self._bus_name = None
