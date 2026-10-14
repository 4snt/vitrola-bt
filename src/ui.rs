//! A janela do Vitrola BT: um tape deck / receiver dos anos 70 desenhado à mão
//! sobre uma tela de 900 x 620 unidades que escala com a janela.

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
const ALT: f32 = 780.0;
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
    agulha: [f32; 2],
    /// Última vez que cada canal encostou no máximo (lâmpada PEAK).
    pico_canal: [Option<Instant>; 2],
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
            pico_canal: [None; 2],
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
            if v.medindo && v.medidas.pico_db[c] > -1.0 {
                self.pico_canal[c] = Some(Instant::now());
            }
        }
        let agora = Instant::now();
        for i in 0..10 {
            let alvo = if v.medindo { v.medidas.bandas_db[i] } else { PISO_DB - 20.0 };
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

        // gabinete: laterais de nogueira e frente de alumínio escovado
        p.rect_filled(area, Rounding::ZERO, rgb(26, 16, 9));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        p.image(self.madeira.id(), r_(0.0, 0.0, 30.0, ALT), uv, Color32::WHITE);
        p.image(self.madeira.id(), r_(LARG - 30.0, 0.0, 30.0, ALT), uv, Color32::WHITE);
        let frente = r_(30.0, 0.0, LARG - 60.0, ALT);
        p.image(self.aluminio.id(), frente, uv, Color32::WHITE);
        d::grad_v(
            &p,
            frente,
            &[(0.0, Color32::from_white_alpha(40)), (0.5, Color32::TRANSPARENT), (1.0, Color32::from_black_alpha(45))],
        );
        p.line_segment(
            [frente.left_top(), frente.right_top()],
            egui::Stroke::new(2.0_f32, Color32::from_white_alpha(200)),
        );
        p.line_segment(
            [frente.left_bottom(), frente.right_bottom()],
            egui::Stroke::new(3.0_f32, Color32::from_black_alpha(90)),
        );
        for (x, y) in [(46.0, 14.0), (LARG - 46.0, 14.0), (46.0, ALT - 14.0), (LARG - 46.0, ALT - 14.0)] {
            d::parafuso(&p, p_(x, y), 4.0 * s);
        }

        // janela 1: VU L/R + espectro
        d::vidro(&p, r_(50.0, 26.0, 800.0, 176.0), s);
        d::vu(&p, r_(64.0, 40.0, 214.0, 148.0), self.agulha[0], "L");
        d::vu(&p, r_(290.0, 40.0, 214.0, 148.0), self.agulha[1], "R");
        for (c, x) in [(0usize, 64.0), (1, 290.0)] {
            let aceso = self.pico_canal[c].is_some_and(|t| t.elapsed() < Duration::from_millis(350));
            d::led(&p, p_(x + 196.0, 54.0), 3.6 * s, rgb(255, 40, 30), aceso);
            p.text(p_(x + 188.0, 54.0), Align2::RIGHT_CENTER, "PEAK", fonte(7.0), Color32::from_rgb(60, 36, 14));
        }
        d::espectro(&p, r_(516.0, 40.0, 322.0, 148.0), &self.bandas, &self.picos);

        // janela 2: fita + display de informações
        d::vidro(&p, r_(50.0, 214.0, 800.0, 216.0), s);
        let modo = v.fluxo.as_ref().map(|f| f.modo.nome().to_uppercase()).unwrap_or_default();
        let titulo = v.aparelho.as_ref().map(|a| a.nome.clone()).unwrap_or_else(|| "SEM FITA".into());
        let sub = match (&v.fluxo, &v.aparelho) {
            (Some(f), _) => format!("sbc {} · {} kbps · {} kHz", f.modo.nome(), f.kbps, f.rate as f32 / 1000.0),
            (None, Some(_)) => "conectada, coloque o disco".into(),
            _ => "ligue o Bluetooth e escolha o PC na vitrola".into(),
        };
        let seg = Self::segundos(v);
        d::fita(
            &p,
            r_(64.0, 226.0, 300.0, 192.0),
            &d::Fita {
                titulo: &titulo,
                subtitulo: &sub,
                angulo: self.angulo,
                progresso: (seg % VOLTA_FITA_S) / VOLTA_FITA_S,
                gravando: v.gravando.is_some(),
            },
        );
        let ap = v.aparelho.as_ref();
        let pc = if !v.bt_disponivel {
            "Bluetooth indisponível".to_string()
        } else {
            format!(
                "{}, {}",
                if v.disfarce { "caixa de som" } else { "computador" },
                if v.visivel { format!("visível \"{}\"", v.alias) } else { "invisível".into() }
            )
        };
        let linhas = [
            ("APARELHO", ap.map(|a| a.nome.clone()).unwrap_or_else(|| "--".into())),
            ("ENDEREÇO", ap.map(|a| a.mac.clone()).unwrap_or_else(|| "--".into())),
            ("CHIP", ap.map(|a| a.chip.clone().unwrap_or_else(|| "lendo...".into())).unwrap_or_else(|| "--".into())),
            ("BLUETOOTH", ap.and_then(|a| a.versao_bt.clone()).unwrap_or_else(|| "--".into())),
            ("CODEC", if v.fluxo.is_some() { format!("SBC {modo}") } else { "--".into() }),
            ("BITPOOL", v.fluxo.as_ref().map(|f| f.bitpool.to_string()).unwrap_or_else(|| "--".into())),
            ("TAXA", v.fluxo.as_ref().map(|f| format!("{} kbps", f.kbps)).unwrap_or_else(|| "--".into())),
            (
                "AMOSTRAGEM",
                v.fluxo
                    .as_ref()
                    .map(|f| format!("{} kHz / 16 bit", f.rate as f32 / 1000.0))
                    .unwrap_or_else(|| "--".into()),
            ),
            ("ESTE PC", pc),
        ];
        d::display(&p, r_(380.0, 226.0, 458.0, 192.0), &linhas);

        // faixa 3: equalizador gráfico e funções de receiver (só a escuta;
        // o REC grava o sinal original)
        self.desenhar_eq(ui, &p, &p_, &r_, s);

        // deck: marca, contador, teclas, alavanca e LEDs
        let tinta = rgb(34, 34, 34);
        d::gravado(&p, p_(64.0, 606.0), Align2::LEFT_TOP, "V I T R O L A   B T", fonte(23.0), tinta);
        d::gravado(&p, p_(64.0, 638.0), Align2::LEFT_TOP, "STEREO CASSETTE DECK", fonte(8.5), tinta);
        d::contador(&p, r_(64.0, 660.0, 150.0, 46.0), seg as u64);
        d::gravado(&p, p_(139.0, 712.0), Align2::CENTER_TOP, "TAPE COUNTER", fonte(8.0), tinta);

        let tem_audio = v.fluxo.is_some();
        let parado = !(v.tocando || v.gravando.is_some());
        let teclas = [
            (Icone::Rec, "REC", 240.0, v.gravando.is_some(), tem_audio, "rec"),
            (Icone::Play, "PLAY", 318.0, v.tocando, tem_audio, "play"),
            (Icone::Stop, "STOP", 396.0, parado, true, "stop"),
            (Icone::Eject, "FITAS", 474.0, false, true, "eject"),
        ];
        for (icone, rotulo, x, travada, habilitada, id) in teclas {
            let r = r_(x, 630.0, 70.0, 76.0);
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
        d::gravado(&p, p_(318.0, 712.0), Align2::LEFT_TOP, "TRAVA MECÂNICA: STOP SOLTA PLAY E REC", fonte(7.5), tinta);

        // alavanca BLUETOOTH
        if self
            .alavanca_pendente
            .is_some_and(|(alvo, t)| alvo == v.receptor_ligado || t.elapsed() > Duration::from_secs(8))
        {
            self.alavanca_pendente = None;
        }
        let ligado = self.alavanca_pendente.map(|(a, _)| a).unwrap_or(v.receptor_ligado);
        d::gravado(&p, p_(640.0, 606.0), Align2::CENTER_TOP, "BLUETOOTH", fonte(9.0), tinta);
        d::gravado(&p, p_(640.0, 622.0), Align2::CENTER_TOP, "ON", fonte(8.0), tinta);
        d::gravado(&p, p_(640.0, 726.0), Align2::CENTER_BOTTOM, "OFF", fonte(8.0), tinta);
        let r_alav = r_(600.0, 632.0, 80.0, 84.0);
        d::alavanca(&p, r_alav, ligado);
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
            ("PLAY", rgb(80, 230, 100), v.tocando),
            ("REC", rgb(255, 59, 47), v.gravando.is_some()),
        ];
        for (i, (rotulo, cor, aceso)) in leds.iter().enumerate() {
            let y = 634.0 + i as f32 * 22.0;
            d::led(&p, p_(722.0, y), 5.0 * s, *cor, *aceso);
            d::gravado(&p, p_(736.0, y), Align2::LEFT_CENTER, rotulo, fonte(8.5), tinta);
        }

        // atalhos gravados no painel
        for (x, rotulo, alvo) in [(784.0, "AJUSTES", 0), (784.0, "REGISTRO", 1)] {
            let y = if alvo == 0 { 634.0 } else { 656.0 };
            let r = r_(x, y - 8.0, 60.0, 16.0);
            let resp =
                ui.interact(r, ui.id().with(rotulo), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            let cor = if resp.hovered() { rgb(150, 30, 20) } else { tinta };
            d::gravado(&p, pos2(r.left(), r.center().y), Align2::LEFT_CENTER, rotulo, fonte(8.5), cor);
            if resp.clicked() {
                if alvo == 0 {
                    self.ver_ajustes = !self.ver_ajustes;
                } else {
                    self.ver_registro = !self.ver_registro;
                }
            }
        }

        // linha de status
        let status = status(v);
        d::gravado(&p, p_(64.0, 750.0), Align2::LEFT_TOP, &status, fonte(10.5), rgb(52, 52, 52));
    }

    fn aplicar_som(&mut self) {
        {
            let mut e = self.c.trava();
            e.som = self.prefs.som.clone();
            e.som_versao += 1;
        }
        self.som_sujo = true;
    }

    fn desenhar_eq(
        &mut self,
        ui: &mut egui::Ui,
        p: &egui::Painter,
        p_: &dyn Fn(f32, f32) -> egui::Pos2,
        r_: &dyn Fn(f32, f32, f32, f32) -> Rect,
        s: f32,
    ) {
        let tinta = rgb(34, 34, 34);
        let fonte = |t: f32| FontId::proportional(t * s);
        let painel = r_(50.0, 440.0, 800.0, 154.0);
        d::grad_v(p, painel, &[(0.0, Color32::from_black_alpha(22)), (1.0, Color32::from_black_alpha(6))]);
        p.rect_stroke(painel, Rounding::same(4.0 * s), egui::Stroke::new(1.0_f32, Color32::from_black_alpha(70)));
        p.line_segment(
            [painel.left_bottom(), painel.right_bottom()],
            egui::Stroke::new(1.0_f32, Color32::from_white_alpha(170)),
        );
        d::gravado(p, p_(64.0, 446.0), Align2::LEFT_TOP, "GRAPHIC EQUALIZER", fonte(8.5), tinta);
        d::gravado(p, p_(520.0, 446.0), Align2::LEFT_TOP, "FILTERS", fonte(8.5), tinta);
        d::gravado(p, p_(806.0, 446.0), Align2::CENTER_TOP, "BALANCE", fonte(8.5), tinta);
        let mut mudou = false;
        let mut soltou = false;

        // FLAT: zera o equalizador
        let r_flat = r_(440.0, 444.0, 60.0, 14.0);
        let resp = ui
            .interact(r_flat, ui.id().with("eq-flat"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let cor = if resp.hovered() { rgb(150, 30, 20) } else { tinta };
        d::gravado(p, pos2(r_flat.right(), r_flat.center().y), Align2::RIGHT_CENTER, "FLAT", fonte(8.5), cor);
        if resp.clicked() {
            self.prefs.som.ganhos = [0.0; 10];
            mudou = true;
            soltou = true;
        }

        let rotulos = ["31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
        for (i, rotulo) in rotulos.iter().enumerate() {
            let r = r_(64.0 + i as f32 * 44.0, 462.0, 40.0, 126.0);
            let resp = ui.interact(r, ui.id().with(("eq", i)), Sense::click_and_drag());
            let resp = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical);
            if resp.double_clicked() {
                self.prefs.som.ganhos[i] = 0.0;
                mudou = true;
                soltou = true;
            } else if resp.dragged() || resp.clicked() {
                if let Some(pt) = resp.interact_pointer_pos() {
                    let k = r.height() / 120.0;
                    let (topo, alt) = (r.top() + 54.0 * k - 46.0 * k, 92.0 * k);
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
            d::cursor_eq(p, r, self.prefs.som.ganhos[i], rotulo);
        }

        let funcoes: [Funcao; 4] = [
            ("RUMBLE", "Corta o ronco do motor e do prato (abaixo de ~35 Hz)", |s| &mut s.rumble),
            ("SCRATCH", "Atenua chiado e estalos do disco (acima de ~8 kHz)", |s| &mut s.scratch),
            ("MONO", "Soma os canais: discos mono antigos ficam com menos ruído", |s| &mut s.mono),
            ("LOUDNESS", "Reforça graves e agudos para ouvir baixo", |s| &mut s.loudness),
        ];
        for (j, (rotulo, dica, campo)) in funcoes.iter().enumerate() {
            let r = r_(522.0 + j as f32 * 60.0, 474.0, 52.0, 62.0);
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
            d::botao_funcao(p, r, rotulo, ligado);
        }
        d::gravado(p, p_(522.0, 552.0), Align2::LEFT_TOP, "SÓ NA ESCUTA: O REC GRAVA O ORIGINAL", fonte(7.0), tinta);

        // BALANCE: arraste para cima/baixo, duplo clique centraliza
        let c = p_(806.0, 512.0);
        let raio = 22.0 * s;
        let r_bal = Rect::from_center_size(c, egui::vec2(raio * 2.6, raio * 2.6));
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
        d::botao_giratorio(p, c, raio, self.prefs.som.balanco, "L", "R");

        if mudou {
            self.aplicar_som();
        }
        if soltou && self.som_sujo {
            self.prefs.salvar();
            self.som_sujo = false;
        }
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
        let animando = v.tocando || v.gravando.is_some() || v.medindo || self.agulha.iter().any(|a| *a > -39.0);
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
