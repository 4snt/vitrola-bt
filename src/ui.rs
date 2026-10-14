//! A janela do Vitrola BT: um sistema hi-fi num rack (DAC Bluetooth, receiver,
//! equalizador gráfico e tape deck), desenhado à mão numa tela de 900 x 944
//! unidades que se ajusta à largura da janela e rola na vertical quando falta altura.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use egui::{
    pos2, vec2, Align2, Color32, FontId, Rect, Rounding, Sense, TextureHandle, TextureOptions, ViewportCommand,
};
use tokio::sync::mpsc::UnboundedSender;

use crate::desenho::{self as d, rgb, Icone};
use crate::estado::{pasta_gravacoes, Aparelho, Comando, Compartilhado, EventoUi, ALIAS};
use crate::medidor::{Medidas, PISO_DB};
use crate::prefs::Prefs;
use crate::sbc;

const LARG: f32 = 900.0;
const ALT: f32 = 944.0;
const VOLTA_FITA_S: f32 = 30.0 * 60.0;

/// Botão de função: rótulo, dica e o campo dos ajustes que ele liga/desliga.
type Funcao = (&'static str, &'static str, fn(&mut crate::dsp::Som) -> &mut bool);

/// Cópia do estado para desenhar um quadro sem segurar a trava.
struct Visao {
    aviso: Option<String>,
    bt_disponivel: bool,
    receptor_ligado: bool,
    visivel: bool,
    disfarce: bool,
    alias: String,
    aparelho: Option<Aparelho>,
    fluxo: Option<sbc::Config>,
    tocando: bool,
    gravando: Option<(PathBuf, Instant)>,
    inicio_play: Option<Instant>,
    medidas: Medidas,
    medidas_saida: Medidas,
    medindo: bool,
    saida_dispositivo: Option<String>,
    saida_volume: Option<(f32, bool)>,
}

pub struct App {
    c: Compartilhado,
    tx: UnboundedSender<Comando>,
    rx_ui: Receiver<EventoUi>,
    prefs: Prefs,
    tem_bandeja: bool,
    aluminio: TextureHandle,
    /// Ponteiros dos VU: entrada (DAC) e saída (receiver).
    agulha: [f32; 2],
    agulha_saida: [f32; 2],
    /// Última vez que cada canal encostou no máximo (lâmpada PEAK): entrada e saída.
    pico_canal: [Option<Instant>; 2],
    pico_saida: [Option<Instant>; 2],
    bandas: [f32; 10],
    picos: [f32; 10],
    pico_t: [Instant; 10],
    angulo: f32,
    ultimo: Instant,
    alavanca_pendente: Option<(bool, Instant)>,
    ver_ajustes: bool,
    ver_registro: bool,
    saindo: bool,
    som_sujo: bool,
    /// Desenvolvimento: VITROLA_CAPTURA=arquivo.png salva uma foto da janela.
    captura: Option<(PathBuf, Instant, bool)>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        c: Compartilhado,
        tx: UnboundedSender<Comando>,
        rx_ui: Receiver<EventoUi>,
        prefs: Prefs,
        tem_bandeja: bool,
    ) -> Self {
        c.repintar.definir(cc.egui_ctx.clone());
        cc.egui_ctx.set_visuals(egui::Visuals::light());
        let aluminio = cc.egui_ctx.load_texture("aluminio", d::textura_aluminio(), TextureOptions::LINEAR);
        let agora = Instant::now();
        App {
            c,
            tx,
            rx_ui,
            prefs,
            tem_bandeja,
            aluminio,
            agulha: [-40.0; 2],
            agulha_saida: [-40.0; 2],
            pico_canal: [None; 2],
            pico_saida: [None; 2],
            bandas: [PISO_DB; 10],
            picos: [PISO_DB; 10],
            pico_t: [agora; 10],
            angulo: 0.0,
            ultimo: agora,
            alavanca_pendente: None,
            ver_ajustes: false,
            ver_registro: false,
            saindo: false,
            som_sujo: false,
            captura: std::env::var_os("VITROLA_CAPTURA").map(|c| (PathBuf::from(c), agora, false)),
        }
    }

    fn visao(&self) -> Visao {
        let e = self.c.trava();
        Visao {
            aviso: e.aviso.clone(),
            bt_disponivel: e.bt_disponivel,
            receptor_ligado: e.receptor_ligado,
            visivel: e.visivel,
            disfarce: e.disfarce_caixa,
            alias: e.alias.clone(),
            aparelho: e.aparelho.clone(),
            fluxo: e.fluxo.clone(),
            tocando: e.tocando,
            gravando: e.gravando.clone(),
            inicio_play: e.inicio_play,
            medidas: e.medidas,
            medidas_saida: e.medidas_saida,
            saida_dispositivo: e.saida_dispositivo.clone(),
            saida_volume: e.saida_volume,
            medindo: e.ultima_medida.is_some_and(|t| t.elapsed() < Duration::from_millis(500)),
        }
    }

    fn enviar(&self, c: Comando) {
        let _ = self.tx.send(c);
    }

    /// Balística: ponteiros de VU (~300 ms) e LEDs que caem a 40 dB/s com pico.
    fn animar(&mut self, v: &Visao, dt: f32) {
        // 0 VU = -14 dBFS: o SBC da vitrola chega quente (média -14..-17 dBFS, picos
        // em 0 dBFS); assim a música fica em -3..0 VU e só o forte entra no vermelho
        let alvo_vu = |db: f32| if v.medindo { db + 14.0 } else { -40.0 };
        let k = 1.0 - (-dt / 0.3).exp();
        for c in 0..2 {
            self.agulha[c] += (alvo_vu(v.medidas.rms_db[c]) - self.agulha[c]) * k;
            self.agulha_saida[c] += (alvo_vu(v.medidas_saida.rms_db[c]) - self.agulha_saida[c]) * k;
            if v.medindo && v.medidas.pico_db[c] > -3.0 {
                self.pico_canal[c] = Some(Instant::now());
            }
            if v.medindo && v.medidas_saida.pico_db[c] > -3.0 {
                self.pico_saida[c] = Some(Instant::now());
            }
        }
        let agora = Instant::now();
        for i in 0..10 {
            // o analisador do equalizador mostra a SAÍDA (depois do EQ)
            let alvo = if v.medindo { v.medidas_saida.bandas_db[i] } else { PISO_DB - 20.0 };
            self.bandas[i] = if alvo > self.bandas[i] { alvo } else { (self.bandas[i] - 40.0 * dt).max(alvo) };
            if self.bandas[i] >= self.picos[i] {
                self.picos[i] = self.bandas[i];
                self.pico_t[i] = agora;
            } else if agora.duration_since(self.pico_t[i]) > Duration::from_millis(1200) {
                self.picos[i] = (self.picos[i] - 25.0 * dt).max(self.bandas[i]);
            }
        }
        if v.tocando || v.gravando.is_some() {
            self.angulo = (self.angulo + dt * 2.6) % (2.0 * std::f32::consts::PI);
        }
    }

    fn segundos(v: &Visao) -> f32 {
        if let Some((_, t)) = &v.gravando {
            return t.elapsed().as_secs_f32();
        }
        v.inicio_play.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0)
    }

    /// Desenha o rack em `area` (largura total da janela; altura ALT * s).
    fn desenhar(&mut self, ui: &mut egui::Ui, v: &Visao, area: Rect, s: f32) {
        let o = pos2(area.center().x - LARG * s / 2.0, area.top());
        let p_ = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
        let r_ = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(p_(x, y), vec2(w * s, h * s));
        let fonte = |t: f32| FontId::proportional(t * s);
        let p = ui.painter().clone();
        let tinta = rgb(34, 34, 34);
        let tema = self.prefs.tema_vu;
        let mut mudou = false;
        let mut soltou = false;

        // fundo e trilhos de rack (no lugar das laterais de madeira)
        p.rect_filled(area, Rounding::ZERO, rgb(14, 14, 16));
        for x in [0.0, LARG - 14.0] {
            let trilho = r_(x, 0.0, 14.0, ALT);
            d::grad_v(&p, trilho, &[(0.0, rgb(70, 70, 74)), (1.0, rgb(42, 42, 46))]);
            let mut y = 10.0;
            while y < ALT - 6.0 {
                p.rect_filled(r_(x + 4.0, y, 6.0, 6.0), Rounding::same(1.0), rgb(12, 12, 14));
                y += 16.0;
            }
        }

        // ===== VB-DAC1 BLUETOOTH DAC (entrada) =====
        self.unidade(&p, r_(18.0, 6.0, 864.0, 230.0), s);
        d::gravado(&p, p_(42.0, 12.0), Align2::LEFT_TOP, "V I T R O L A   B T", fonte(15.0), tinta);
        d::gravado(&p, p_(232.0, 17.0), Align2::LEFT_TOP, "VB-DAC1  ·  BLUETOOTH DAC  ·  INPUT", fonte(7.5), tinta);
        d::vidro(&p, r_(38.0, 36.0, 560.0, 158.0), s);
        d::vu(&p, r_(46.0, 44.0, 270.0, 142.0), self.agulha[0], "IN L", tema);
        d::vu(&p, r_(320.0, 44.0, 270.0, 142.0), self.agulha[1], "IN R", tema);
        for (c, x) in [(0usize, 46.0), (1, 320.0)] {
            let aceso = self.pico_canal[c].is_some_and(|t| t.elapsed() < Duration::from_millis(350));
            d::led(&p, p_(x + 252.0, 56.0), 3.0 * s, rgb(255, 40, 30), aceso);
        }
        let ap = v.aparelho.as_ref();
        let pc = if !v.bt_disponivel {
            "sem Bluetooth".to_string()
        } else {
            format!(
                "{}{}",
                if v.disfarce { "caixa de som" } else { "computador" },
                if v.visivel { format!(", visível \"{}\"", v.alias) } else { String::new() }
            )
        };
        let entrada = [
            ("APARELHO", ap.map(|a| a.nome.clone()).unwrap_or_else(|| "--".into())),
            ("ENDEREÇO", ap.map(|a| self.mac_visivel(&a.mac)).unwrap_or_else(|| "--".into())),
            ("CHIP", ap.map(|a| a.chip.clone().unwrap_or_else(|| "lendo...".into())).unwrap_or_else(|| "--".into())),
            ("BLUETOOTH", ap.and_then(|a| a.versao_bt.clone()).unwrap_or_else(|| "--".into())),
            (
                "CODEC",
                v.fluxo
                    .as_ref()
                    .map(|f| format!("SBC {}", f.modo.nome().to_uppercase()))
                    .unwrap_or_else(|| "--".into()),
            ),
            (
                "TAXA",
                v.fluxo
                    .as_ref()
                    .map(|f| format!("{} kbps  bp{}  {}k", f.kbps, f.bitpool, f.rate / 1000))
                    .unwrap_or_else(|| "--".into()),
            ),
            ("ESTE PC", pc),
        ];
        d::display(&p, r_(610.0, 44.0, 252.0, 142.0), &entrada);
        let em_dual = v.fluxo.as_ref().is_some_and(|f| f.modo == sbc::Modo::Dual);
        for (i, (rotulo, cor, aceso)) in [
            ("BT LINK", rgb(64, 180, 255), v.aparelho.is_some()),
            ("SBC LOCK", rgb(255, 180, 58), v.fluxo.is_some()),
            ("DUAL 396", rgb(80, 230, 100), em_dual),
        ]
        .iter()
        .enumerate()
        {
            let x = 616.0 + i as f32 * 84.0;
            d::led(&p, p_(x, 208.0), 4.0 * s, *cor, *aceso);
            d::gravado(&p, p_(x + 10.0, 208.0), Align2::LEFT_CENTER, rotulo, fonte(7.0), tinta);
        }
        d::gravado(
            &p,
            p_(42.0, 202.0),
            Align2::LEFT_TOP,
            "SBC DECODER · 48 kHz · JITTER BUFFER 250 ms",
            fonte(7.0),
            tinta,
        );

        // ===== VB-7000 STEREO RECEIVER (saída) =====
        let y0 = 242.0;
        self.unidade(&p, r_(18.0, y0, 864.0, 318.0), s);
        d::gravado(
            &p,
            p_(42.0, y0 + 8.0),
            Align2::LEFT_TOP,
            "VB-7000  ·  STEREO RECEIVER  ·  OUTPUT",
            fonte(7.5),
            tinta,
        );
        d::vidro(&p, r_(38.0, y0 + 30.0, 560.0, 158.0), s);
        d::vu(&p, r_(46.0, y0 + 38.0, 270.0, 142.0), self.agulha_saida[0], "OUT L", tema);
        d::vu(&p, r_(320.0, y0 + 38.0, 270.0, 142.0), self.agulha_saida[1], "OUT R", tema);
        for (c, x) in [(0usize, 46.0), (1, 320.0)] {
            let aceso = self.pico_saida[c].is_some_and(|t| t.elapsed() < Duration::from_millis(350));
            d::led(&p, p_(x + 252.0, y0 + 50.0), 3.0 * s, rgb(255, 40, 30), aceso);
        }
        // segunda tela: o que sai
        let som = &self.prefs.som;
        let eq_ativas = som.ganhos.iter().filter(|g| g.abs() > 0.01).count();
        let filtros: Vec<&str> =
            [(som.rumble, "RUMBLE"), (som.scratch, "SCRATCH"), (som.mono, "MONO"), (som.loudness, "LOUD")]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, n)| *n)
                .collect();
        let vol_db =
            if som.volume <= 0.0 { "MUDO".to_string() } else { format!("{:.0} dB", (som.volume - 1.0) * 60.0) };
        let saida = [
            ("SAÍDA", v.saida_dispositivo.clone().unwrap_or_else(|| "--".into())),
            (
                "SISTEMA",
                v.saida_volume
                    .map(|(vol, mudo)| if mudo { "mudo".into() } else { format!("volume {:.0}%", vol * 100.0) })
                    .unwrap_or_else(|| "--".into()),
            ),
            ("VOLUME", format!("{:.0}%  ({vol_db})", som.volume * 100.0)),
            ("EQ", if eq_ativas == 0 { "FLAT".into() } else { format!("{eq_ativas} bandas ajustadas") }),
            ("FILTROS", if filtros.is_empty() { "--".into() } else { filtros.join(" ") }),
            (
                "BALANCE",
                if som.balanco.abs() < 0.01 {
                    "CENTRO".into()
                } else {
                    format!("{} {:.0}%", if som.balanco < 0.0 { "L" } else { "R" }, som.balanco.abs() * 100.0)
                },
            ),
            (
                "ESTADO",
                match (&v.gravando, v.tocando) {
                    (Some(_), _) => "GRAVANDO FLAC".into(),
                    (None, true) => "TOCANDO".into(),
                    _ if v.fluxo.is_some() => "PARADO".into(),
                    _ => "--".into(),
                },
            ),
        ];
        d::display(&p, r_(610.0, y0 + 38.0, 252.0, 142.0), &saida);

        // fileira de controles (embaixo)
        let yc = y0 + 196.0;
        p.line_segment(
            [p_(38.0, yc - 2.0), p_(862.0, yc - 2.0)],
            egui::Stroke::new(1.0_f32, Color32::from_black_alpha(50)),
        );
        if self
            .alavanca_pendente
            .is_some_and(|(alvo, t)| alvo == v.receptor_ligado || t.elapsed() > Duration::from_secs(8))
        {
            self.alavanca_pendente = None;
        }
        let ligado = self.alavanca_pendente.map(|(a, _)| a).unwrap_or(v.receptor_ligado);
        d::gravado(&p, p_(62.0, yc + 6.0), Align2::CENTER_TOP, "BLUETOOTH", fonte(7.0), tinta);
        let r_alav = r_(40.0, yc + 18.0, 44.0, 76.0);
        d::alavanca(&p, r_alav, ligado);
        d::gravado(&p, p_(90.0, yc + 22.0), Align2::LEFT_TOP, "ON", fonte(7.0), tinta);
        d::gravado(&p, p_(90.0, yc + 90.0), Align2::LEFT_BOTTOM, "OFF", fonte(7.0), tinta);
        let resp = ui
            .interact(r_alav, ui.id().with("alavanca"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() && self.alavanca_pendente.is_none() {
            let novo = !ligado;
            self.alavanca_pendente = Some((novo, Instant::now()));
            self.prefs.bluetooth_ligado = novo;
            self.prefs.salvar();
            self.enviar(if novo { Comando::LigarBluetooth } else { Comando::DesligarBluetooth });
        }
        for (i, (rotulo, cor, aceso)) in [
            ("POWER", rgb(255, 180, 58), ligado),
            ("LINK", rgb(64, 180, 255), v.aparelho.is_some()),
            ("STEREO", rgb(255, 180, 58), v.fluxo.is_some()),
        ]
        .iter()
        .enumerate()
        {
            let y = yc + 30.0 + i as f32 * 20.0;
            d::led(&p, p_(128.0, y), 4.0 * s, *cor, *aceso);
            d::gravado(&p, p_(138.0, y), Align2::LEFT_CENTER, rotulo, fonte(7.0), tinta);
        }
        let funcoes: [Funcao; 4] = [
            ("RUMBLE", "Corta o ronco do motor e do prato (abaixo de ~35 Hz)", |s| &mut s.rumble),
            ("SCRATCH", "Atenua chiado e estalos do disco (acima de ~8 kHz)", |s| &mut s.scratch),
            ("MONO", "Soma os canais: discos mono antigos ficam com menos ruído", |s| &mut s.mono),
            ("LOUDNESS", "Reforça graves e agudos para ouvir baixo", |s| &mut s.loudness),
        ];
        for (j, (rotulo, dica, campo)) in funcoes.iter().enumerate() {
            let r = r_(210.0 + j as f32 * 62.0, yc + 24.0, 54.0, 60.0);
            let resp = ui
                .interact(r, ui.id().with(("func", j)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(*dica);
            if resp.clicked() {
                let v = campo(&mut self.prefs.som);
                *v = !*v;
                mudou = true;
                soltou = true;
            }
            let ligado = *campo(&mut self.prefs.som);
            d::botao_funcao(&p, r, rotulo, ligado);
        }
        let r_hifi = r_(470.0, yc + 24.0, 66.0, 60.0);
        let resp = ui
            .interact(r_hifi, ui.id().with("hifi"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Renegocia o SBC dual channel (~396 kbps). A lâmpada acende quando ele está ativo.");
        if resp.clicked() {
            self.enviar(Comando::HiFi);
        }
        d::botao_funcao(&p, r_hifi, "HI-FI", em_dual || resp.is_pointer_button_down_on());
        for (y, rotulo, alvo) in [(yc + 40.0, "AJUSTES", 0), (yc + 62.0, "REGISTRO", 1)] {
            let r = r_(556.0, y - 8.0, 70.0, 16.0);
            let resp =
                ui.interact(r, ui.id().with(rotulo), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let cor = if resp.hovered() { rgb(150, 30, 20) } else { tinta };
            d::gravado(&p, pos2(r.left(), r.center().y), Align2::LEFT_CENTER, rotulo, fonte(8.0), cor);
            if resp.clicked() {
                if alvo == 0 {
                    self.ver_ajustes = !self.ver_ajustes;
                } else {
                    self.ver_registro = !self.ver_registro;
                }
            }
        }
        // VOLUME e BALANCE
        d::gravado(&p, p_(710.0, yc + 4.0), Align2::CENTER_TOP, "VOLUME", fonte(7.5), tinta);
        let c_vol = p_(710.0, yc + 58.0);
        let r_vol = Rect::from_center_size(c_vol, vec2(96.0 * s, 96.0 * s));
        let resp = ui
            .interact(r_vol, ui.id().with("volume"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resp.double_clicked() {
            self.prefs.som.volume = 1.0;
            mudou = true;
            soltou = true;
        } else if resp.dragged() {
            self.prefs.som.volume = (self.prefs.som.volume - resp.drag_delta().y / (160.0 * s)).clamp(0.0, 1.0);
            mudou = true;
        }
        soltou |= resp.drag_stopped();
        d::botao_giratorio(&p, c_vol, 34.0 * s, self.prefs.som.volume * 2.0 - 1.0, "MIN", "MAX");
        d::gravado(&p, p_(822.0, yc + 18.0), Align2::CENTER_TOP, "BALANCE", fonte(7.5), tinta);
        let c_bal = p_(822.0, yc + 62.0);
        let r_bal = Rect::from_center_size(c_bal, vec2(60.0 * s, 60.0 * s));
        let resp = ui
            .interact(r_bal, ui.id().with("balance"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if resp.double_clicked() {
            self.prefs.som.balanco = 0.0;
            mudou = true;
            soltou = true;
        } else if resp.dragged() {
            let b = (self.prefs.som.balanco - resp.drag_delta().y / (80.0 * s)).clamp(-1.0, 1.0);
            self.prefs.som.balanco = if b.abs() < 0.04 { 0.0 } else { b };
            mudou = true;
        }
        soltou |= resp.drag_stopped();
        d::botao_giratorio(&p, c_bal, 22.0 * s, self.prefs.som.balanco, "L", "R");

        // ===== VB-EQ10 GRAPHIC EQUALIZER (espectro da saída) =====
        let ye = 566.0;
        self.unidade(&p, r_(18.0, ye, 864.0, 172.0), s);
        d::gravado(
            &p,
            p_(42.0, ye + 6.0),
            Align2::LEFT_TOP,
            "VB-EQ10  ·  GRAPHIC EQUALIZER  ·  OUTPUT SPECTRUM",
            fonte(7.5),
            tinta,
        );
        d::vidro(&p, r_(38.0, ye + 22.0, 300.0, 140.0), s);
        d::espectro(&p, r_(44.0, ye + 28.0, 288.0, 128.0), &self.bandas, &self.picos);
        let rotulos = ["31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
        for (i, rotulo) in rotulos.iter().enumerate() {
            let r = r_(360.0 + i as f32 * 44.0, ye + 20.0, 40.0, 142.0);
            let resp = ui
                .interact(r, ui.id().with(("eq", i)), Sense::click_and_drag())
                .on_hover_cursor(egui::CursorIcon::ResizeVertical);
            if resp.double_clicked() {
                self.prefs.som.ganhos[i] = 0.0;
                mudou = true;
                soltou = true;
            } else if resp.dragged() || resp.clicked() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    let k = r.height() / 120.0;
                    let (topo, alt) = (r.top() + 8.0 * k, 92.0 * k);
                    let mut g = ((topo + alt - pt.y) / alt * 24.0 - 12.0).clamp(-12.0, 12.0);
                    if g.abs() < 0.6 {
                        g = 0.0; // trava no centro
                    }
                    g = (g * 2.0).round() / 2.0;
                    if g != self.prefs.som.ganhos[i] {
                        self.prefs.som.ganhos[i] = g;
                        mudou = true;
                    }
                }
            }
            soltou |= resp.drag_stopped() || resp.clicked();
            d::cursor_eq(&p, r, self.prefs.som.ganhos[i], rotulo);
        }
        let r_flat = r_(812.0, ye + 56.0, 44.0, 56.0);
        let resp = ui
            .interact(r_flat, ui.id().with("eq-flat"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Zera o equalizador");
        if resp.clicked() {
            self.prefs.som.ganhos = [0.0; 10];
            mudou = true;
            soltou = true;
        }
        let eq_ativo = self.prefs.som.ganhos.iter().any(|g| g.abs() > 0.01);
        d::botao_funcao(&p, r_flat, "FLAT", !eq_ativo || resp.is_pointer_button_down_on());

        // ===== VB-C70 STEREO CASSETTE DECK (compacto) =====
        let yd = 744.0;
        self.unidade(&p, r_(18.0, yd, 864.0, 172.0), s);
        d::gravado(&p, p_(42.0, yd + 6.0), Align2::LEFT_TOP, "VB-C70  ·  STEREO CASSETTE DECK", fonte(7.5), tinta);
        d::gravado(
            &p,
            p_(858.0, yd + 6.0),
            Align2::RIGHT_TOP,
            "DIRECT FLAC RECORDING · 48 kHz · 16 BIT",
            fonte(7.0),
            tinta,
        );
        d::vidro(&p, r_(38.0, yd + 22.0, 222.0, 142.0), s);
        let titulo = v.aparelho.as_ref().map(|a| a.nome.clone()).unwrap_or_else(|| "SEM FITA".into());
        let sub = match (&v.fluxo, &v.aparelho) {
            (Some(f), _) => format!("sbc {} · {} kbps", f.modo.nome(), f.kbps),
            (None, Some(_)) => "conectada, coloque o disco".into(),
            _ => "aguardando a vitrola".into(),
        };
        let seg = Self::segundos(v);
        d::fita(
            &p,
            r_(44.0, yd + 28.0, 210.0, 130.0),
            &d::Fita {
                titulo: &titulo,
                subtitulo: &sub,
                angulo: self.angulo,
                progresso: (seg % VOLTA_FITA_S) / VOLTA_FITA_S,
                gravando: v.gravando.is_some(),
            },
        );
        d::contador(&p, r_(280.0, yd + 30.0, 132.0, 42.0), seg as u64);
        d::gravado(&p, p_(346.0, yd + 76.0), Align2::CENTER_TOP, "TAPE COUNTER", fonte(7.0), tinta);
        for (i, (rotulo, cor, aceso)) in
            [("PLAY", rgb(80, 230, 100), v.tocando), ("REC", rgb(255, 59, 47), v.gravando.is_some())].iter().enumerate()
        {
            let x = 290.0 + i as f32 * 64.0;
            d::led(&p, p_(x, yd + 106.0), 4.5 * s, *cor, *aceso);
            d::gravado(&p, p_(x + 10.0, yd + 106.0), Align2::LEFT_CENTER, rotulo, fonte(7.5), tinta);
        }
        let tem_audio = v.fluxo.is_some();
        let parado = !(v.tocando || v.gravando.is_some());
        let teclas = [
            (Icone::Rec, "REC", 450.0, v.gravando.is_some(), tem_audio, "rec"),
            (Icone::Play, "PLAY", 528.0, v.tocando, tem_audio, "play"),
            (Icone::Stop, "STOP", 606.0, parado, true, "stop"),
            (Icone::Eject, "FITAS", 684.0, false, true, "eject"),
        ];
        for (icone, rotulo, x, travada, habilitada, id) in teclas {
            let r = r_(x, yd + 34.0, 70.0, 76.0);
            let resp = ui.interact(r, ui.id().with(id), if habilitada { Sense::click() } else { Sense::hover() });
            let segurando = habilitada && resp.is_pointer_button_down_on();
            d::tecla(&p, r, icone, rotulo, travada || segurando, habilitada);
            let resp = if habilitada { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
            if resp.clicked() {
                match id {
                    "rec" => self.enviar(Comando::Rec),
                    "play" => self.enviar(Comando::Play),
                    "stop" => self.enviar(Comando::Stop),
                    _ => abrir_pasta(),
                }
            }
        }
        d::gravado(
            &p,
            p_(450.0, yd + 120.0),
            Align2::LEFT_TOP,
            "TRAVA MECÂNICA: STOP SOLTA PLAY E REC",
            fonte(7.0),
            tinta,
        );

        // status embaixo do rack
        let status = status(v);
        p.text(p_(22.0, ALT - 20.0), Align2::LEFT_TOP, &status, fonte(10.0), rgb(196, 190, 176));

        if mudou {
            self.aplicar_som();
        }
        if soltou && self.som_sujo {
            self.prefs.salvar();
            self.som_sujo = false;
        }
    }

    /// Frente de um aparelho do rack: alumínio escovado, bisel e parafusos de rack.
    fn unidade(&self, p: &egui::Painter, r: Rect, s: f32) {
        p.rect_filled(r.translate(vec2(0.0, 3.0 * s)), Rounding::same(3.0 * s), Color32::from_black_alpha(140));
        p.image(self.aluminio.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        d::grad_v(
            p,
            r,
            &[(0.0, Color32::from_white_alpha(48)), (0.45, Color32::TRANSPARENT), (1.0, Color32::from_black_alpha(40))],
        );
        p.line_segment([r.left_top(), r.right_top()], egui::Stroke::new(1.5_f32, Color32::from_white_alpha(220)));
        p.line_segment([r.left_bottom(), r.right_bottom()], egui::Stroke::new(2.0_f32, Color32::from_black_alpha(110)));
        for (dx, dy) in [
            (10.0, 10.0),
            (r.width() / s - 10.0, 10.0),
            (10.0, r.height() / s - 10.0),
            (r.width() / s - 10.0, r.height() / s - 10.0),
        ] {
            d::parafuso(p, r.min + vec2(dx * s, dy * s), 3.6 * s);
        }
    }

    fn aplicar_som(&mut self) {
        {
            let mut e = self.c.trava();
            e.som = self.prefs.som.clone();
            e.som_versao += 1;
        }
        self.som_sujo = true;
    }

    fn janelas(&mut self, ctx: &egui::Context) {
        let mut ver = self.ver_ajustes;
        egui::Window::new("Ajustes").open(&mut ver).resizable(false).collapsible(false).show(ctx, |ui| {
            let mut mudou = false;
            if ui.checkbox(&mut self.prefs.tocar_auto, "Tocar automaticamente quando a vitrola conectar").changed() {
                mudou = true;
                let _ = self.tx.send(Comando::TocarAuto(self.prefs.tocar_auto));
            }
            ui.add_enabled_ui(self.tem_bandeja, |ui| {
                let r = ui
                    .checkbox(&mut self.prefs.minimizar_bandeja, "Fechar a janela minimiza (o app continua na bandeja)")
                    .on_disabled_hover_text("Este desktop não tem bandeja do sistema (StatusNotifier)");
                mudou |= r.changed();
            });
            ui.horizontal(|ui| {
                ui.label("Cor dos VU:");
                for t in [d::TemaVu::Verde, d::TemaVu::Azul, d::TemaVu::Ambar] {
                    mudou |= ui.radio_value(&mut self.prefs.tema_vu, t, t.nome()).changed();
                }
            });
            if mudou {
                self.prefs.salvar();
            }
            ui.separator();
            if ui.button("Abrir pasta de gravações").clicked() {
                abrir_pasta();
            }
            ui.label(format!("Gravações em {}", pasta_gravacoes().display()));
        });
        self.ver_ajustes = ver;

        let mut ver = self.ver_registro;
        egui::Window::new("Registro").open(&mut ver).default_size([560.0, 300.0]).show(ctx, |ui| {
            let linhas: Vec<String> = self.c.trava().log.iter().cloned().collect();
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for l in linhas {
                    ui.label(egui::RichText::new(l).monospace().size(12.0));
                }
            });
        });
        self.ver_registro = ver;
    }
}

impl App {
    /// Nas fotos de divulgação o MAC aparece mascarado (identifica o aparelho).
    fn mac_visivel(&self, mac: &str) -> String {
        if self.captura.is_some() {
            format!("{}:XX:XX:XX", mac.split(':').take(3).collect::<Vec<_>>().join(":"))
        } else {
            mac.to_string()
        }
    }

    fn capturar(&mut self, ctx: &egui::Context) {
        let Some((caminho, inicio, pedida)) = &mut self.captura else { return };
        if inicio.elapsed() < Duration::from_millis(300) {
            // janela grande, para a foto ter resolução
            ctx.send_viewport_cmd(ViewportCommand::Maximized(true));
        }
        let atraso = std::env::var("VITROLA_CAPTURA_ATRASO").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.5);
        if !*pedida && inicio.elapsed() > Duration::from_secs_f32(atraso) {
            ctx.send_viewport_cmd(ViewportCommand::Screenshot);
            *pedida = true;
        }
        let imagem = ctx.input(|i| {
            i.raw.events.iter().find_map(|e| {
                if let egui::Event::Screenshot { image, .. } = e {
                    Some(image.clone())
                } else {
                    None
                }
            })
        });
        if let Some(img) = imagem {
            let salvar = || -> std::io::Result<()> {
                let arq = std::io::BufWriter::new(std::fs::File::create(&*caminho)?);
                let mut enc = png::Encoder::new(arq, img.size[0] as u32, img.size[1] as u32);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                let mut w = enc.write_header().map_err(std::io::Error::other)?;
                w.write_image_data(img.as_raw()).map_err(std::io::Error::other)
            };
            match salvar() {
                Ok(()) => eprintln!("captura salva em {}", caminho.display()),
                Err(e) => eprintln!("captura falhou: {e}"),
            }
            *pedida = true;
            *inicio = Instant::now() + Duration::from_secs(3600);
        }
    }
}

fn status(v: &Visao) -> String {
    if let Some(a) = &v.aviso {
        return a.clone();
    }
    match (&v.fluxo, &v.aparelho) {
        (Some(f), _) => {
            let mut s = format!("Recebendo SBC {} a {} kbps.", f.modo.nome(), f.kbps);
            if let Some((c, _)) = &v.gravando {
                s.push_str(&format!(" Gravando em {}.", c.display()));
            } else if !v.tocando {
                s.push_str(" Parado: aperte PLAY para ouvir.");
            }
            s
        }
        (None, Some(a)) if v.receptor_ligado => format!("{} conectada. Coloque o disco para tocar.", a.nome),
        (None, Some(a)) => format!("{} conectada, mas o BLUETOOTH do deck está desligado.", a.nome),
        _ if !v.receptor_ligado => "Ligue a alavanca BLUETOOTH para receber a vitrola.".into(),
        _ if v.visivel => format!("Na vitrola, ligue o Bluetooth e escolha \"{ALIAS}\"."),
        _ => "Aguardando a vitrola reconectar.".into(),
    }
}

fn abrir_pasta() {
    let pasta = pasta_gravacoes();
    let _ = std::fs::create_dir_all(&pasta);
    let _ = std::process::Command::new("xdg-open").arg(&pasta).spawn();
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(ev) = self.rx_ui.try_recv() {
            match ev {
                EventoUi::Mostrar => {
                    ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(ViewportCommand::Focus);
                }
                EventoUi::Sair => {
                    self.saindo = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
        if ctx.input(|i| i.viewport().close_requested())
            && !self.saindo
            && self.prefs.minimizar_bandeja
            && self.tem_bandeja
        {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        let agora = Instant::now();
        let dt = agora.duration_since(self.ultimo).as_secs_f32().min(0.1);
        self.ultimo = agora;
        let v = self.visao();
        self.animar(&v, dt);
        egui::CentralPanel::default().frame(egui::Frame::none().fill(rgb(14, 14, 16))).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                // ocupa toda a largura; o rack escala por ela e rola quando falta altura
                let largura = ui.available_width();
                let mut s = (largura / LARG).clamp(0.45, 2.2);
                if self.captura.is_some() {
                    // foto de desenvolvimento: o rack inteiro cabe na janela
                    s = s.min(ui.available_height() / ALT);
                }
                let (area, _) = ui.allocate_exact_size(vec2(largura, ALT * s), Sense::hover());
                self.desenhar(ui, &v, area, s);
            });
        });
        self.janelas(ctx);
        self.capturar(ctx);
        let animando = v.tocando
            || v.gravando.is_some()
            || v.medindo
            || self.agulha.iter().chain(&self.agulha_saida).any(|a| *a > -39.0);
        ctx.request_repaint_after(if animando { Duration::from_millis(16) } else { Duration::from_millis(250) });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // Fecha de verdade: grava o FLAC, para o som, solta a vitrola
        let (tx, rx) = std::sync::mpsc::channel();
        if self.tx.send(Comando::Sair(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(5));
        }
    }
}
