//! A janela do Vitrola BT: um sistema hi-fi empilhado num gabinete de nogueira
//! (DAC Bluetooth, receiver, equalizador gráfico e tape deck), desenhado à mão
//! sobre uma tela de 900 x 906 unidades que escala com a janela.

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
const ALT: f32 = 906.0;
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
}

pub struct App {
    c: Compartilhado,
    tx: UnboundedSender<Comando>,
    rx_ui: Receiver<EventoUi>,
    prefs: Prefs,
    tem_bandeja: bool,
    aluminio: TextureHandle,
    madeira: TextureHandle,
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
        let madeira = cc.egui_ctx.load_texture("madeira", d::textura_madeira(), TextureOptions::LINEAR);
        let agora = Instant::now();
        App {
            c,
            tx,
            rx_ui,
            prefs,
            tem_bandeja,
            aluminio,
            madeira,
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
            medindo: e.ultima_medida.is_some_and(|t| t.elapsed() < Duration::from_millis(500)),
        }
    }

    fn enviar(&self, c: Comando) {
        let _ = self.tx.send(c);
    }

    /// Balística: ponteiros de VU (~300 ms) e LEDs que caem a 40 dB/s com pico.
    fn animar(&mut self, v: &Visao, dt: f32) {
        let alvo_vu = |db: f32| if v.medindo { db + 18.0 } else { -40.0 };
        let k = 1.0 - (-dt / 0.3).exp();
        for c in 0..2 {
            self.agulha[c] += (alvo_vu(v.medidas.rms_db[c]) - self.agulha[c]) * k;
            self.agulha_saida[c] += (alvo_vu(v.medidas_saida.rms_db[c]) - self.agulha_saida[c]) * k;
            if v.medindo && v.medidas.pico_db[c] > -1.0 {
                self.pico_canal[c] = Some(Instant::now());
            }
            if v.medindo && v.medidas_saida.pico_db[c] > -1.0 {
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

    fn desenhar(&mut self, ui: &mut egui::Ui, v: &Visao) {
        let area = ui.max_rect();
        let s = (area.width() / LARG).min(area.height() / ALT);
        let o = area.center() - vec2(LARG * s / 2.0, ALT * s / 2.0);
        let p_ = |x: f32, y: f32| pos2(o.x + x * s, o.y + y * s);
        let r_ = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(p_(x, y), vec2(w * s, h * s));
        let fonte = |t: f32| FontId::proportional(t * s);
        let p = ui.painter().clone();
        let tinta = rgb(34, 34, 34);

        // gabinete de nogueira com o rack de três aparelhos
        p.rect_filled(area, Rounding::ZERO, rgb(18, 12, 7));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        for r in [r_(0.0, 0.0, 28.0, ALT - 28.0), r_(LARG - 28.0, 0.0, 28.0, ALT - 28.0)] {
            p.image(self.madeira.id(), r, uv, Color32::WHITE);
        }
        for r in [r_(0.0, 0.0, LARG, 12.0), r_(0.0, ALT - 48.0, LARG, 20.0)] {
            p.image(self.madeira.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 0.04)), Color32::WHITE);
        }
        p.rect_filled(r_(28.0, 12.0, LARG - 56.0, ALT - 60.0), Rounding::ZERO, rgb(10, 8, 6));

        // ===== VB-DAC1 BLUETOOTH DAC (entrada) =====
        self.unidade(&p, r_(34.0, 16.0, 832.0, 160.0), s);
        d::gravado(&p, p_(60.0, 22.0), Align2::LEFT_TOP, "V I T R O L A   B T", fonte(15.0), tinta);
        d::gravado(&p, p_(250.0, 27.0), Align2::LEFT_TOP, "VB-DAC1  ·  BLUETOOTH DAC  ·  INPUT", fonte(7.5), tinta);
        d::vidro(&p, r_(56.0, 46.0, 560.0, 122.0), s);
        d::vu(&p, r_(64.0, 54.0, 140.0, 106.0), self.agulha[0], "IN L");
        d::vu(&p, r_(210.0, 54.0, 140.0, 106.0), self.agulha[1], "IN R");
        for (c, x) in [(0usize, 64.0), (1, 210.0)] {
            let aceso = self.pico_canal[c].is_some_and(|t| t.elapsed() < Duration::from_millis(350));
            d::led(&p, p_(x + 126.0, 64.0), 2.6 * s, rgb(255, 40, 30), aceso);
        }

        // ===== VB-7000 STEREO RECEIVER =====
        let y0 = 182.0; // receiver
        self.unidade(&p, r_(34.0, y0, 832.0, 252.0), s);
        d::gravado(
            &p,
            p_(60.0, y0 + 8.0),
            Align2::LEFT_TOP,
            "VB-7000  ·  STEREO RECEIVER  ·  OUTPUT",
            fonte(7.5),
            tinta,
        );
        d::vidro(&p, r_(56.0, y0 + 32.0, 560.0, 158.0), s);
        d::vu(&p, r_(64.0, y0 + 40.0, 270.0, 142.0), self.agulha_saida[0], "OUT L");
        d::vu(&p, r_(338.0, y0 + 40.0, 270.0, 142.0), self.agulha_saida[1], "OUT R");
        for (c, x) in [(0usize, 64.0), (1, 338.0)] {
            let aceso = self.pico_saida[c].is_some_and(|t| t.elapsed() < Duration::from_millis(350));
            d::led(&p, p_(x + 252.0, y0 + 52.0), 3.0 * s, rgb(255, 40, 30), aceso);
            p.text(p_(x + 245.0, y0 + 52.0), Align2::RIGHT_CENTER, "PEAK", fonte(6.0), Color32::from_rgb(60, 36, 14));
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
        let linhas = [
            ("APARELHO", ap.map(|a| a.nome.clone()).unwrap_or_else(|| "--".into())),
            ("ENDEREÇO", ap.map(|a| a.mac.clone()).unwrap_or_else(|| "--".into())),
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
        d::display(&p, r_(356.0, 54.0, 252.0, 106.0), &linhas);
        let em_dual = v.fluxo.as_ref().is_some_and(|f| f.modo == sbc::Modo::Dual);
        for (i, (rotulo, cor, aceso)) in [
            ("BT LINK", rgb(64, 180, 255), v.aparelho.is_some()),
            ("SBC LOCK", rgb(255, 180, 58), v.fluxo.is_some()),
            ("DUAL 396", rgb(80, 230, 100), em_dual),
        ]
        .iter()
        .enumerate()
        {
            let y = 62.0 + i as f32 * 20.0;
            d::led(&p, p_(640.0, y), 4.0 * s, *cor, *aceso);
            d::gravado(&p, p_(652.0, y), Align2::LEFT_CENTER, rotulo, fonte(7.5), tinta);
        }
        d::gravado(&p, p_(640.0, 128.0), Align2::LEFT_TOP, "SBC DECODER · 48 kHz", fonte(7.0), tinta);
        d::gravado(&p, p_(640.0, 142.0), Align2::LEFT_TOP, "JITTER BUFFER 250 ms", fonte(7.0), tinta);

        // VOLUME e BALANCE
        d::gravado(&p, p_(690.0, y0 + 34.0), Align2::CENTER_TOP, "VOLUME", fonte(8.0), tinta);
        let c_vol = p_(690.0, y0 + 104.0);
        let r_vol = Rect::from_center_size(c_vol, vec2(100.0 * s, 100.0 * s));
        let resp = ui
            .interact(r_vol, ui.id().with("volume"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::ResizeVertical);
        let mut mudou = false;
        let mut soltou = false;
        if resp.double_clicked() {
            self.prefs.som.volume = 1.0;
            mudou = true;
            soltou = true;
        } else if resp.dragged() {
            self.prefs.som.volume = (self.prefs.som.volume - resp.drag_delta().y / (160.0 * s)).clamp(0.0, 1.0);
            mudou = true;
        }
        soltou |= resp.drag_stopped();
        d::botao_giratorio(&p, c_vol, 38.0 * s, self.prefs.som.volume * 2.0 - 1.0, "MIN", "MAX");
        d::gravado(&p, p_(808.0, y0 + 54.0), Align2::CENTER_TOP, "BALANCE", fonte(8.0), tinta);
        let c_bal = p_(808.0, y0 + 104.0);
        let r_bal = Rect::from_center_size(c_bal, vec2(64.0 * s, 64.0 * s));
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
        d::botao_giratorio(&p, c_bal, 24.0 * s, self.prefs.som.balanco, "L", "R");

        // linha de baixo: alavanca BLUETOOTH, LEDs, funções e HI-FI
        if self
            .alavanca_pendente
            .is_some_and(|(alvo, t)| alvo == v.receptor_ligado || t.elapsed() > Duration::from_secs(8))
        {
            self.alavanca_pendente = None;
        }
        let ligado = self.alavanca_pendente.map(|(a, _)| a).unwrap_or(v.receptor_ligado);
        let r_alav = r_(62.0, y0 + 196.0, 40.0, 52.0);
        d::alavanca(&p, r_alav, ligado);
        d::gravado(&p, p_(106.0, y0 + 198.0), Align2::LEFT_TOP, "BT ON", fonte(7.0), tinta);
        d::gravado(&p, p_(106.0, y0 + 246.0), Align2::LEFT_BOTTOM, "OFF", fonte(7.0), tinta);
        let resp = ui
            .interact(r_alav, ui.id().with("alavanca"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.clicked() && self.alavanca_pendente.is_none() {
            let novo = !ligado;
            self.alavanca_pendente = Some((novo, Instant::now()));
            self.enviar(if novo { Comando::LigarBluetooth } else { Comando::DesligarBluetooth });
        }
        let leds = [
            ("POWER", rgb(255, 180, 58), ligado),
            ("LINK", rgb(64, 180, 255), v.aparelho.is_some()),
            ("STEREO", rgb(255, 180, 58), v.fluxo.is_some()),
        ];
        for (i, (rotulo, cor, aceso)) in leds.iter().enumerate() {
            let y = y0 + 206.0 + i as f32 * 17.0;
            d::led(&p, p_(150.0, y), 4.0 * s, *cor, *aceso);
            d::gravado(&p, p_(160.0, y), Align2::LEFT_CENTER, rotulo, fonte(7.0), tinta);
        }
        let funcoes: [Funcao; 4] = [
            ("RUMBLE", "Corta o ronco do motor e do prato (abaixo de ~35 Hz)", |s| &mut s.rumble),
            ("SCRATCH", "Atenua chiado e estalos do disco (acima de ~8 kHz)", |s| &mut s.scratch),
            ("MONO", "Soma os canais: discos mono antigos ficam com menos ruído", |s| &mut s.mono),
            ("LOUDNESS", "Reforça graves e agudos para ouvir baixo", |s| &mut s.loudness),
        ];
        for (j, (rotulo, dica, campo)) in funcoes.iter().enumerate() {
            let r = r_(250.0 + j as f32 * 62.0, y0 + 196.0, 54.0, 52.0);
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
        let r_hifi = r_(510.0, y0 + 196.0, 64.0, 52.0);
        let resp = ui
            .interact(r_hifi, ui.id().with("hifi"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Renegocia o SBC dual channel (~396 kbps). A lâmpada acende quando ele está ativo.");
        if resp.clicked() {
            self.enviar(Comando::HiFi);
        }
        d::botao_funcao(&p, r_hifi, "HI-FI", em_dual || resp.is_pointer_button_down_on());
        for (y, rotulo, alvo) in [(y0 + 206.0, "AJUSTES", 0), (y0 + 228.0, "REGISTRO", 1)] {
            let r = r_(770.0, y - 8.0, 80.0, 16.0);
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

        // ===== VB-EQ10 GRAPHIC EQUALIZER / SPECTRUM ANALYZER =====
        self.unidade(&p, r_(34.0, 440.0, 832.0, 172.0), s);
        d::gravado(
            &p,
            p_(60.0, 446.0),
            Align2::LEFT_TOP,
            "VB-EQ10  ·  GRAPHIC EQUALIZER  ·  OUTPUT SPECTRUM",
            fonte(7.5),
            tinta,
        );
        d::vidro(&p, r_(56.0, 462.0, 300.0, 140.0), s);
        d::espectro(&p, r_(62.0, 468.0, 288.0, 128.0), &self.bandas, &self.picos);
        let rotulos = ["31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
        for (i, rotulo) in rotulos.iter().enumerate() {
            let r = r_(374.0 + i as f32 * 43.0, 460.0, 40.0, 142.0);
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
        let r_flat = r_(814.0, 496.0, 40.0, 52.0);
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

        // ===== VB-C70 STEREO CASSETTE DECK =====
        self.unidade(&p, r_(34.0, 618.0, 832.0, 238.0), s);
        d::gravado(&p, p_(60.0, 624.0), Align2::LEFT_TOP, "VB-C70  ·  STEREO CASSETTE DECK", fonte(7.5), tinta);
        d::gravado(&p, p_(850.0, 624.0), Align2::RIGHT_TOP, "DIRECT FLAC RECORDING", fonte(7.5), tinta);
        d::vidro(&p, r_(56.0, 640.0, 320.0, 208.0), s);
        let titulo = v.aparelho.as_ref().map(|a| a.nome.clone()).unwrap_or_else(|| "SEM FITA".into());
        let sub = match (&v.fluxo, &v.aparelho) {
            (Some(f), _) => format!("sbc {} · {} kbps · {} kHz", f.modo.nome(), f.kbps, f.rate as f32 / 1000.0),
            (None, Some(_)) => "conectada, coloque o disco".into(),
            _ => "ligue o Bluetooth e escolha o PC na vitrola".into(),
        };
        let seg = Self::segundos(v);
        d::fita(
            &p,
            r_(64.0, 648.0, 304.0, 192.0),
            &d::Fita {
                titulo: &titulo,
                subtitulo: &sub,
                angulo: self.angulo,
                progresso: (seg % VOLTA_FITA_S) / VOLTA_FITA_S,
                gravando: v.gravando.is_some(),
            },
        );
        d::contador(&p, r_(400.0, 648.0, 150.0, 46.0), seg as u64);
        d::gravado(&p, p_(475.0, 700.0), Align2::CENTER_TOP, "TAPE COUNTER", fonte(7.5), tinta);
        for (i, (rotulo, cor, aceso)) in
            [("PLAY", rgb(80, 230, 100), v.tocando), ("REC", rgb(255, 59, 47), v.gravando.is_some())].iter().enumerate()
        {
            let y = 660.0 + i as f32 * 22.0;
            d::led(&p, p_(578.0, y), 4.5 * s, *cor, *aceso);
            d::gravado(&p, p_(590.0, y), Align2::LEFT_CENTER, rotulo, fonte(8.0), tinta);
        }
        let tem_audio = v.fluxo.is_some();
        let parado = !(v.tocando || v.gravando.is_some());
        let teclas = [
            (Icone::Rec, "REC", 400.0, v.gravando.is_some(), tem_audio, "rec"),
            (Icone::Play, "PLAY", 478.0, v.tocando, tem_audio, "play"),
            (Icone::Stop, "STOP", 556.0, parado, true, "stop"),
            (Icone::Eject, "FITAS", 634.0, false, true, "eject"),
        ];
        for (icone, rotulo, x, travada, habilitada, id) in teclas {
            let r = r_(x, 732.0, 70.0, 76.0);
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
        d::gravado(&p, p_(400.0, 818.0), Align2::LEFT_TOP, "TRAVA MECÂNICA: STOP SOLTA PLAY E REC", fonte(7.0), tinta);
        d::gravado(&p, p_(850.0, 836.0), Align2::RIGHT_TOP, "48 kHz · 16 BIT · LOSSLESS", fonte(7.0), tinta);

        // etiqueta de status abaixo do rack
        let status = status(v);
        p.text(p_(36.0, ALT - 22.0), Align2::LEFT_TOP, &status, fonte(10.0), rgb(196, 186, 168));

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
    fn capturar(&mut self, ctx: &egui::Context) {
        let Some((caminho, inicio, pedida)) = &mut self.captura else { return };
        if !*pedida && inicio.elapsed() > Duration::from_millis(2500) {
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
        egui::CentralPanel::default().frame(egui::Frame::none()).show(ctx, |ui| self.desenhar(ui, &v));
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
