//! Desenho skeuomórfico do deck: texturas procedurais (alumínio escovado,
//! nogueira), VU meters de mostrador âmbar, analisador em LEDs, fita cassete,
//! teclas de piano com trava, alavanca, LEDs, contador e display fluorescente.
//! Tudo vetorial, sem fontes de ícones nem emoji.

use std::f32::consts::PI;

use egui::{pos2, vec2, Align2, Color32, ColorImage, FontId, Mesh, Painter, Pos2, Rect, Rounding, Shape, Stroke};

use crate::medidor::CENTROS;

pub const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn cor_em(paradas: &[(f32, Color32)], t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    for w in paradas.windows(2) {
        let ((t0, c0), (t1, c1)) = (w[0], w[1]);
        if t <= t1 {
            return mix(c0, c1, if t1 > t0 { (t - t0) / (t1 - t0) } else { 0.0 });
        }
    }
    paradas.last().map(|p| p.1).unwrap_or(Color32::BLACK)
}

/// Gradiente vertical com várias paradas.
pub fn grad_v(p: &Painter, r: Rect, paradas: &[(f32, Color32)]) {
    let mut m = Mesh::default();
    for (i, &(t, c)) in paradas.iter().enumerate() {
        let y = r.top() + r.height() * t;
        m.colored_vertex(pos2(r.left(), y), c);
        m.colored_vertex(pos2(r.right(), y), c);
        if i > 0 {
            let b = (i as u32) * 2;
            m.add_triangle(b - 2, b - 1, b);
            m.add_triangle(b - 1, b, b + 1);
        }
    }
    p.add(Shape::mesh(m));
}

/// Gradiente radial aproximado por uma malha (cores por vértice).
pub fn grad_radial(p: &Painter, r: Rect, centro: Pos2, raio: f32, paradas: &[(f32, Color32)]) {
    let (nx, ny) = (28u32, 14u32);
    let mut m = Mesh::default();
    for j in 0..=ny {
        for i in 0..=nx {
            let pos = pos2(r.left() + r.width() * i as f32 / nx as f32, r.top() + r.height() * j as f32 / ny as f32);
            m.colored_vertex(pos, cor_em(paradas, pos.distance(centro) / raio));
        }
    }
    for j in 0..ny {
        for i in 0..nx {
            let a = j * (nx + 1) + i;
            m.add_triangle(a, a + 1, a + nx + 1);
            m.add_triangle(a + 1, a + nx + 2, a + nx + 1);
        }
    }
    p.add(Shape::mesh(m));
}

fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^ (x >> 16)
}

fn ruido(x: u32, y: u32, semente: u32) -> f32 {
    (hash(x.wrapping_mul(73_856_093) ^ y.wrapping_mul(19_349_663) ^ semente) & 0xFFFF) as f32 / 65535.0
}

/// Alumínio escovado: listras horizontais finas com variação lenta.
pub fn textura_aluminio() -> ColorImage {
    let (w, h) = (1024usize, 384usize);
    let mut linhas = vec![0f32; h];
    for (y, l) in linhas.iter_mut().enumerate() {
        *l = ruido(0, y as u32, 11) - 0.5;
    }
    let suave: Vec<f32> =
        (0..h).map(|y| (linhas[y.saturating_sub(1)] + 2.0 * linhas[y] + linhas[(y + 1).min(h - 1)]) / 4.0).collect();
    let mut px = Vec::with_capacity(w * h * 4);
    for (y, &s) in suave.iter().enumerate() {
        let fase = ruido(1, y as u32, 5) * std::f32::consts::TAU;
        for x in 0..w {
            let lenta = (x as f32 * 0.006 + fase).sin() * 3.0;
            let fino = (ruido((x / 4) as u32, y as u32, 3) - 0.5) * 7.0;
            let v = (206.0 + s * 26.0 + lenta + fino).clamp(0.0, 255.0);
            px.extend_from_slice(&[v as u8, v as u8, (v - 3.0).max(0.0) as u8, 255]);
        }
    }
    ColorImage::from_rgba_unmultiplied([w, h], &px)
}

/// Texto gravado no metal: sombra clara 1 px abaixo, tinta escura por cima.
pub fn gravado(p: &Painter, pos: Pos2, align: Align2, txt: &str, fonte: FontId, cor: Color32) {
    p.text(pos + vec2(0.0, 1.0), align, txt, fonte.clone(), Color32::from_white_alpha(150));
    p.text(pos, align, txt, fonte, cor);
}

pub fn parafuso(p: &Painter, c: Pos2, r: f32) {
    p.circle_filled(c + vec2(0.0, r * 0.25), r, Color32::from_black_alpha(60));
    p.circle_filled(c, r, rgb(168, 167, 162));
    p.circle_stroke(c, r, Stroke::new(1.0_f32, rgb(92, 91, 87)));
    let d = vec2(r * 0.6, -r * 0.6);
    p.line_segment([c - d, c + d], Stroke::new(r * 0.28, rgb(84, 83, 79)));
}

/// Janela de vidro com moldura prateada.
pub fn vidro(p: &Painter, r: Rect, k: f32) -> Rect {
    grad_v(p, r, &[(0.0, rgb(238, 237, 233)), (1.0, rgb(118, 117, 113))]);
    let dentro = r.shrink(4.0 * k);
    grad_v(p, dentro, &[(0.0, rgb(27, 27, 28)), (1.0, rgb(5, 5, 6))]);
    p.rect_stroke(dentro, Rounding::ZERO, Stroke::new(1.0_f32, Color32::BLACK));
    dentro
}

// ---------- VU meter ----------

fn angulo_vu(vu: f32) -> f32 {
    let v = vu.clamp(-20.0, 3.0);
    let frac = (10f32.powf(v / 20.0) - 0.1) / (10f32.powf(3.0 / 20.0) - 0.1);
    (-50.0 + 100.0 * frac).to_radians()
}

/// Iluminação do mostrador dos VU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TemaVu {
    /// Verde iluminado (o padrão).
    #[default]
    Verde,
    /// Azul com escala clara, como receivers clássicos de alta fidelidade.
    Azul,
    /// Creme com lâmpada âmbar, vintage.
    Ambar,
}

impl TemaVu {
    pub fn nome(self) -> &'static str {
        match self {
            TemaVu::Verde => "Verde iluminado",
            TemaVu::Azul => "Azul clássico",
            TemaVu::Ambar => "Âmbar vintage",
        }
    }

    /// (paradas do mostrador, tinta da escala, vermelho da zona de pico)
    fn paleta(self) -> ([(f32, Color32); 3], Color32, Color32) {
        match self {
            TemaVu::Verde => (
                [(0.0, rgb(196, 255, 206)), (0.5, rgb(64, 214, 118)), (1.0, rgb(8, 82, 44))],
                rgb(8, 34, 18),
                rgb(196, 22, 14),
            ),
            TemaVu::Azul => (
                [(0.0, rgb(120, 205, 255)), (0.5, rgb(18, 112, 228)), (1.0, rgb(4, 34, 96))],
                rgb(236, 246, 255),
                rgb(255, 92, 76),
            ),
            TemaVu::Ambar => (
                [(0.0, rgb(255, 231, 164)), (0.5, rgb(247, 190, 88)), (1.0, rgb(128, 72, 16))],
                rgb(28, 18, 10),
                rgb(186, 24, 14),
            ),
        }
    }
}

/// VU analógico de receiver dos anos 70: mostrador creme iluminado por lâmpada
/// âmbar, escala -20..+3 e ponteiro. `valor` em VU (0 VU ~ -18 dBFS).
pub fn vu(p: &Painter, r: Rect, valor: f32, canal: &str, tema: TemaVu) {
    let (paradas, tinta, vermelho) = tema.paleta();
    let k = r.width() / 214.0;
    p.rect_filled(r, Rounding::same(5.0 * k), rgb(9, 9, 10));
    let face = r.shrink(6.0 * k);
    let pc = p.with_clip_rect(face);
    grad_radial(&pc, face, pos2(face.center().x, face.bottom()), face.width() * 0.85, &paradas);
    let piv = pos2(face.center().x, face.bottom() + 4.0 * k);
    let raio = face.height() * 0.80;
    let ponto = |vu: f32, rr: f32| {
        let a = angulo_vu(vu);
        pos2(piv.x + rr * a.sin(), piv.y - rr * a.cos())
    };
    let arco =
        |de: f32, ate: f32, rr: f32| (0..=40).map(|i| ponto(de + (ate - de) * i as f32 / 40.0, rr)).collect::<Vec<_>>();
    pc.add(Shape::line(arco(-20.0, 0.0, raio), Stroke::new(1.3 * k, tinta)));
    pc.add(Shape::line(arco(0.0, 3.0, raio), Stroke::new(3.2 * k, vermelho)));
    for vu in [-20.0f32, -10.0, -7.0, -5.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0] {
        let cor = if vu > 0.0 { vermelho } else { tinta };
        pc.line_segment([ponto(vu, raio + 1.0 * k), ponto(vu, raio + 7.0 * k)], Stroke::new(1.2 * k, cor));
        if [-20.0, -10.0, -5.0, -3.0, 0.0, 3.0].contains(&vu) {
            let txt = if vu == 0.0 { "0".to_string() } else { format!("{:+}", vu as i32) };
            pc.text(ponto(vu, raio + 15.0 * k), Align2::CENTER_CENTER, txt, FontId::proportional(9.5 * k), cor);
        }
    }
    pc.text(pos2(piv.x, piv.y - raio * 0.42), Align2::CENTER_CENTER, "VU", FontId::proportional(13.0 * k), tinta);
    pc.text(
        pos2(face.left() + 8.0 * k, face.bottom() - 8.0 * k),
        Align2::LEFT_BOTTOM,
        canal,
        FontId::proportional(10.0 * k),
        tinta,
    );
    // ponteiro com sombra
    let ponta = ponto(valor, raio + 8.0 * k);
    let sombra = vec2(2.0 * k, 2.0 * k);
    pc.line_segment([piv + sombra, ponta + sombra], Stroke::new(1.6 * k, Color32::from_black_alpha(70)));
    let agulha = if tema == TemaVu::Azul { rgb(250, 250, 250) } else { rgb(18, 12, 6) };
    pc.line_segment([piv, ponta], Stroke::new(1.5 * k, agulha));
    pc.circle_filled(piv, 9.0 * k, rgb(14, 12, 10));
    // reflexo do vidro: degradê suave, sem borda
    grad_v(
        &pc,
        Rect::from_min_size(face.min, vec2(face.width(), face.height() * 0.45)),
        &[(0.0, Color32::from_white_alpha(34)), (1.0, Color32::TRANSPARENT)],
    );
}

// ---------- analisador de espectro ----------

pub const SEGMENTOS: usize = 16;
const TOPO_DB: f32 = -10.0;
const PISO_DB: f32 = -60.0;

fn segmentos_acesos(db: f32) -> usize {
    let frac = (db - PISO_DB) / (TOPO_DB - PISO_DB);
    (frac * SEGMENTOS as f32).round().clamp(0.0, SEGMENTOS as f32) as usize
}

pub fn espectro(p: &Painter, r: Rect, valores: &[f32; 10], picos: &[f32; 10]) {
    let k = r.width() / 322.0;
    p.rect_filled(r, Rounding::same(5.0 * k), rgb(4, 4, 5));
    let (margem, rodape) = (10.0 * k, 16.0 * k);
    let larg = (r.width() - 2.0 * margem) / CENTROS.len() as f32;
    let alt = (r.height() - margem - rodape) / SEGMENTOS as f32;
    let rotulos = ["31", "63", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"];
    for i in 0..CENTROS.len() {
        let x = r.left() + margem + i as f32 * larg + 3.0 * k;
        let acesos = segmentos_acesos(valores[i]);
        let pico = segmentos_acesos(picos[i]);
        for s in 0..SEGMENTOS {
            let y = r.bottom() - rodape - (s + 1) as f32 * alt + 1.0;
            let cor = if s >= SEGMENTOS - 2 {
                rgb(255, 58, 46)
            } else if s >= SEGMENTOS - 5 {
                rgb(255, 184, 40)
            } else {
                rgb(92, 236, 112)
            };
            let aceso = s < acesos || (pico > 0 && s + 1 == pico);
            let c = if aceso { cor } else { cor.gamma_multiply(0.09) };
            p.rect_filled(Rect::from_min_size(pos2(x, y), vec2(larg - 6.0 * k, alt - 2.0 * k)), Rounding::same(1.0), c);
        }
        p.text(
            pos2(x + (larg - 6.0 * k) / 2.0, r.bottom() - 4.0 * k),
            Align2::CENTER_BOTTOM,
            rotulos[i],
            FontId::proportional(9.0 * k),
            Color32::from_white_alpha(150),
        );
    }
}

// ---------- fita cassete ----------

pub struct Fita<'a> {
    pub titulo: &'a str,
    pub subtitulo: &'a str,
    pub angulo: f32,
    pub progresso: f32,
    pub gravando: bool,
}

fn caber(p: &Painter, txt: &str, fonte: &FontId, largura: f32) -> String {
    let mede = |t: &str| p.layout_no_wrap(t.to_string(), fonte.clone(), Color32::WHITE).size().x;
    if mede(txt) <= largura {
        return txt.to_string();
    }
    let mut chars: Vec<char> = txt.chars().collect();
    while chars.pop().is_some() {
        let t = format!("{}…", chars.iter().collect::<String>().trim_end());
        if mede(&t) <= largura {
            return t;
        }
    }
    "…".into()
}

pub fn fita(p: &Painter, r: Rect, f: &Fita) {
    // proporção de uma cassete real: 100 x 63
    let w = r.width().min(r.height() * 100.0 / 63.0);
    let h = w * 63.0 / 100.0;
    let o = pos2(r.center().x - w / 2.0, r.center().y - h / 2.0);
    let u = w / 100.0;
    let q = |x: f32, y: f32| pos2(o.x + x * u, o.y + y * u);
    let rr = |x: f32, y: f32, ww: f32, hh: f32| Rect::from_min_size(q(x, y), vec2(ww * u, hh * u));

    // carcaça
    p.rect_filled(rr(0.0, 0.0, 100.0, 63.0), Rounding::same(3.2 * u), rgb(34, 34, 37));
    p.rect_stroke(rr(0.0, 0.0, 100.0, 63.0), Rounding::same(3.2 * u), Stroke::new(0.5 * u, rgb(78, 78, 83)));
    for (x, y) in [(3.5, 3.5), (96.5, 3.5), (3.5, 59.5), (96.5, 59.5), (50.0, 59.5)] {
        p.circle_filled(q(x, y), 1.1 * u, rgb(108, 108, 112));
        p.line_segment([q(x - 0.7, y), q(x + 0.7, y)], Stroke::new(0.3 * u, rgb(48, 48, 50)));
    }
    // etiqueta de papel com faixas retrô
    p.rect_filled(rr(6.0, 5.0, 88.0, 42.0), Rounding::same(1.6 * u), rgb(238, 228, 205));
    for (i, c) in [rgb(217, 84, 46), rgb(237, 153, 51), rgb(242, 204, 89)].iter().enumerate() {
        p.rect_filled(rr(6.0, 34.0 + i as f32 * 2.2, 88.0, 2.2), Rounding::ZERO, *c);
    }
    p.text(q(9.0, 7.0), Align2::LEFT_TOP, "A", FontId::proportional(7.0 * u), rgb(38, 38, 38));
    for y in [14.5, 20.5] {
        p.line_segment(
            [q(16.0, y), q(91.0, y)],
            Stroke::new(0.25 * u, Color32::from_rgba_unmultiplied(89, 115, 166, 90)),
        );
    }
    let tinta = rgb(28, 32, 78);
    let fonte_t = FontId::proportional(5.2 * u);
    let fonte_s = FontId::proportional(3.5 * u);
    p.text(q(17.0, 8.6), Align2::LEFT_TOP, caber(p, f.titulo, &fonte_t, 72.0 * u), fonte_t, tinta);
    p.text(q(17.0, 16.4), Align2::LEFT_TOP, caber(p, f.subtitulo, &fonte_s, 72.0 * u), fonte_s, tinta);
    if f.gravando {
        p.circle_filled(q(88.5, 10.5), 1.6 * u, rgb(217, 31, 31));
    }

    // janela, rolos de fita e carretéis
    let janela = rr(22.0, 22.0, 56.0, 15.0);
    p.rect_filled(janela, Rounding::same(7.5 * u), rgb(15, 15, 18));
    let pj = p.with_clip_rect(janela.shrink(0.6 * u));
    let (rmin, rmax) = (4.6, 10.5);
    let prog = f.progresso.clamp(0.0, 1.0);
    for (cx, raio) in [(32.0, rmin + (rmax - rmin) * (1.0 - prog)), (68.0, rmin + (rmax - rmin) * prog)] {
        pj.circle_filled(q(cx, 29.5), raio * u, rgb(62, 41, 26));
    }
    for cx in [32.0, 68.0] {
        let c = q(cx, 29.5);
        p.circle_filled(c, 4.3 * u, rgb(236, 236, 236));
        p.circle_filled(c, 2.4 * u, rgb(15, 15, 18));
        for i in 0..6 {
            let a = f.angulo + i as f32 * PI / 3.0;
            let (s, co) = a.sin_cos();
            let rot = |x: f32, y: f32| pos2(c.x + (x * co - y * s) * u, c.y + (x * s + y * co) * u);
            p.add(Shape::convex_polygon(
                vec![rot(-0.45, -2.5), rot(0.45, -2.5), rot(0.45, -1.3), rot(-0.45, -1.3)],
                rgb(236, 236, 236),
                Stroke::NONE,
            ));
        }
    }
    p.rect_filled(rr(44.0, 25.0, 12.0, 9.0), Rounding::ZERO, Color32::from_rgba_unmultiplied(140, 140, 155, 40));
    for i in 0..5 {
        let x = 45.0 + i as f32 * 2.5;
        let topo = if i % 2 == 1 { 31.8 } else { 31.0 };
        p.line_segment([q(x, 33.0), q(x, topo)], Stroke::new(0.2 * u, Color32::from_white_alpha(130)));
    }

    // base trapezoidal com furos de guia e feltro
    p.add(Shape::convex_polygon(
        vec![q(18.0, 63.0), q(23.0, 50.0), q(77.0, 50.0), q(82.0, 63.0)],
        rgb(46, 46, 49),
        Stroke::NONE,
    ));
    for x in [30.0, 70.0] {
        p.circle_filled(q(x, 56.5), 1.6 * u, rgb(10, 10, 10));
    }
    for x in [40.0, 57.0] {
        p.rect_filled(rr(x, 54.5, 3.0, 4.0), Rounding::ZERO, rgb(10, 10, 10));
    }
    p.rect_filled(rr(47.5, 55.0, 5.0, 3.0), Rounding::ZERO, rgb(140, 107, 64));
}

// ---------- teclas, alavanca, LEDs, contador, display ----------

#[derive(Clone, Copy)]
pub enum Icone {
    Rec,
    Play,
    Stop,
    Eject,
}

/// Tecla de piano. `r` inclui a espessura; afundada desce e perde a espessura.
pub fn tecla(p: &Painter, r: Rect, icone: Icone, rotulo: &str, afundada: bool, habilitada: bool) {
    let k = r.width() / 70.0;
    let prof = 8.0 * k;
    let alt_face = r.height() - prof;
    let desce = if afundada { prof - 2.0 * k } else { 0.0 };
    // corpo / espessura
    p.rect_filled(
        Rect::from_min_max(pos2(r.left(), r.top() + desce + alt_face * 0.4), r.max),
        Rounding::same(3.0 * k),
        rgb(90, 89, 85),
    );
    p.rect_filled(
        Rect::from_min_max(
            pos2(r.left() + 2.0 * k, r.bottom() - 2.0 * k),
            pos2(r.right() - 2.0 * k, r.bottom() + 2.0 * k),
        ),
        Rounding::same(2.0 * k),
        Color32::from_black_alpha(if afundada { 30 } else { 70 }),
    );
    let face = Rect::from_min_size(pos2(r.left(), r.top() + desce), vec2(r.width(), alt_face));
    if afundada {
        grad_v(p, face, &[(0.0, rgb(178, 177, 172)), (1.0, rgb(210, 209, 204))]);
    } else {
        grad_v(p, face, &[(0.0, rgb(253, 253, 251)), (0.6, rgb(215, 214, 209)), (1.0, rgb(176, 175, 170))]);
    }
    p.rect_stroke(face, Rounding::same(2.0 * k), Stroke::new(1.0_f32, rgb(104, 103, 99)));
    if !afundada {
        p.line_segment(
            [pos2(face.left() + 2.0 * k, face.top() + 1.5 * k), pos2(face.right() - 2.0 * k, face.top() + 1.5 * k)],
            Stroke::new(1.0_f32, Color32::from_white_alpha(200)),
        );
    }
    let c = pos2(face.center().x, face.top() + alt_face * 0.38);
    let tinta = if habilitada { rgb(38, 38, 38) } else { rgb(130, 130, 128) };
    match icone {
        Icone::Rec => {
            p.circle_filled(c, 6.0 * k, if habilitada { rgb(180, 36, 28) } else { rgb(170, 130, 128) });
        }
        Icone::Play => {
            p.add(Shape::convex_polygon(
                vec![c + vec2(-5.0 * k, -7.0 * k), c + vec2(7.0 * k, 0.0), c + vec2(-5.0 * k, 7.0 * k)],
                tinta,
                Stroke::NONE,
            ));
        }
        Icone::Stop => {
            p.rect_filled(Rect::from_center_size(c, vec2(11.0 * k, 11.0 * k)), Rounding::ZERO, tinta);
        }
        Icone::Eject => {
            p.add(Shape::convex_polygon(
                vec![c + vec2(-7.0 * k, 2.0 * k), c + vec2(0.0, -7.0 * k), c + vec2(7.0 * k, 2.0 * k)],
                tinta,
                Stroke::NONE,
            ));
            p.rect_filled(
                Rect::from_center_size(c + vec2(0.0, 5.5 * k), vec2(14.0 * k, 2.6 * k)),
                Rounding::ZERO,
                tinta,
            );
        }
    }
    p.text(
        pos2(face.center().x, face.bottom() - 6.0 * k),
        Align2::CENTER_BOTTOM,
        rotulo,
        FontId::proportional(9.0 * k),
        tinta,
    );
}

/// Alavanca de metal (interruptor de aparelho antigo): para cima = ligado.
pub fn alavanca(p: &Painter, r: Rect, ligado: bool) {
    let k = r.height() / 110.0;
    let c = r.center();
    p.circle_filled(c + vec2(0.0, 2.0 * k), 16.0 * k, Color32::from_black_alpha(60));
    grad_radial(
        &p.with_clip_rect(Rect::from_center_size(c, vec2(30.0 * k, 30.0 * k))),
        Rect::from_center_size(c, vec2(30.0 * k, 30.0 * k)),
        c + vec2(-5.0 * k, -5.0 * k),
        22.0 * k,
        &[(0.0, rgb(255, 255, 255)), (0.6, rgb(170, 170, 168)), (1.0, rgb(70, 70, 68))],
    );
    let hex: Vec<Pos2> = (0..6)
        .map(|i| {
            let a = PI / 6.0 + i as f32 * PI / 3.0;
            c + vec2(a.cos(), a.sin()) * 16.0 * k
        })
        .collect();
    p.add(Shape::closed_line(hex, Stroke::new(1.2 * k, rgb(80, 80, 78))));
    let ponta = c + vec2(3.0 * k, if ligado { -34.0 } else { 34.0 } * k);
    p.line_segment(
        [c + vec2(2.0 * k, 2.0 * k), ponta + vec2(2.0 * k, 2.0 * k)],
        Stroke::new(7.0 * k, Color32::from_black_alpha(70)),
    );
    p.line_segment([c, ponta], Stroke::new(7.0 * k, rgb(96, 96, 94)));
    p.line_segment([c, ponta], Stroke::new(4.0 * k, rgb(214, 214, 210)));
    p.circle_filled(ponta, 6.5 * k, rgb(110, 110, 108));
    p.circle_filled(ponta + vec2(-1.5 * k, -1.5 * k), 4.5 * k, rgb(236, 236, 233));
    p.circle_filled(c, 5.0 * k, rgb(60, 60, 58));
}

pub fn led(p: &Painter, c: Pos2, r: f32, cor: Color32, aceso: bool) {
    if aceso {
        for i in (1..=4).rev() {
            p.circle_filled(c, r * (1.0 + i as f32 * 0.55), cor.gamma_multiply(0.07));
        }
    }
    p.circle_filled(c, r, if aceso { cor } else { mix(cor, rgb(20, 12, 10), 0.8) });
    p.circle_stroke(c, r, Stroke::new(1.0_f32, rgb(45, 45, 45)));
    p.circle_filled(c + vec2(-r * 0.35, -r * 0.35), r * 0.3, Color32::from_white_alpha(if aceso { 170 } else { 60 }));
}

/// Contador de fita: rodas de dígitos brancos sobre preto.
pub fn contador(p: &Painter, r: Rect, segundos: u64) {
    p.rect_filled(r.expand(3.0), Rounding::same(4.0), rgb(128, 127, 122));
    p.rect_filled(r, Rounding::same(3.0), rgb(8, 8, 8));
    let txt = format!("{:02}{:02}", (segundos / 60) % 100, segundos % 60);
    let cel = r.width() / 4.6;
    for (i, ch) in txt.chars().enumerate() {
        let x = r.left() + cel * 0.2 + i as f32 * cel + if i >= 2 { cel * 0.4 } else { 0.0 };
        let celula = Rect::from_min_size(pos2(x, r.top() + 3.0), vec2(cel * 0.86, r.height() - 6.0));
        grad_v(
            p,
            celula,
            &[(0.0, rgb(52, 52, 52)), (0.25, rgb(18, 18, 18)), (0.75, rgb(18, 18, 18)), (1.0, rgb(52, 52, 52))],
        );
        p.text(
            celula.center(),
            Align2::CENTER_CENTER,
            ch.to_string(),
            FontId::monospace(r.height() * 0.6),
            rgb(242, 242, 238),
        );
    }
    let sep = pos2(r.left() + cel * 2.3, r.center().y);
    p.circle_filled(sep + vec2(0.0, -r.height() * 0.15), 1.8, rgb(242, 242, 238));
    p.circle_filled(sep + vec2(0.0, r.height() * 0.15), 1.8, rgb(242, 242, 238));
}

/// Display fluorescente (VFD) verde-azulado.
pub fn display(p: &Painter, r: Rect, linhas: &[(&str, String)]) {
    let k = r.height() / 190.0;
    p.rect_filled(r, Rounding::same(4.0 * k), rgb(5, 11, 10));
    p.rect_stroke(r, Rounding::same(4.0 * k), Stroke::new(2.0_f32, rgb(141, 140, 135)));
    let fonte = FontId::monospace(12.5 * k);
    let alt = (r.height() - 16.0 * k) / linhas.len().max(1) as f32;
    for (i, (chave, valor)) in linhas.iter().enumerate() {
        let y = r.top() + 8.0 * k + i as f32 * alt + alt / 2.0;
        p.text(pos2(r.left() + 12.0 * k, y), Align2::LEFT_CENTER, *chave, fonte.clone(), rgb(46, 140, 116));
        let v = caber(p, valor, &fonte, r.width() - 140.0 * k);
        // leve brilho do fósforo
        p.text(
            pos2(r.left() + 128.0 * k, y + 0.6),
            Align2::LEFT_CENTER,
            &v,
            fonte.clone(),
            Color32::from_rgba_unmultiplied(125, 242, 207, 50),
        );
        p.text(pos2(r.left() + 128.0 * k, y), Align2::LEFT_CENTER, v, fonte.clone(), rgb(125, 242, 207));
    }
    grad_v(
        &p.with_clip_rect(r.shrink(2.0)),
        Rect::from_min_size(r.min, vec2(r.width(), r.height() * 0.4)),
        &[(0.0, Color32::from_white_alpha(14)), (1.0, Color32::TRANSPARENT)],
    );
}

// ---------- equalizador gráfico e funções ----------

/// Cursor deslizante de equalizador: trilho escuro, escala e capa prateada.
/// `ganho` em dB (-12..+12).
pub fn cursor_eq(p: &Painter, r: Rect, ganho: f32, rotulo: &str) {
    let k = r.height() / 120.0;
    let trilho = Rect::from_center_size(pos2(r.center().x, r.top() + 54.0 * k), vec2(4.0 * k, 92.0 * k));
    // escala: +12, 0, -12
    for (i, db) in [12.0f32, 6.0, 0.0, -6.0, -12.0].iter().enumerate() {
        let y = trilho.top() + trilho.height() * i as f32 / 4.0;
        let largura = if *db == 0.0 { 10.0 } else { 6.0 } * k;
        p.line_segment(
            [pos2(trilho.left() - largura - 2.0 * k, y), pos2(trilho.left() - 2.0 * k, y)],
            Stroke::new(1.0_f32, rgb(52, 52, 52)),
        );
        p.line_segment(
            [pos2(trilho.right() + 2.0 * k, y), pos2(trilho.right() + largura + 2.0 * k, y)],
            Stroke::new(1.0_f32, rgb(52, 52, 52)),
        );
    }
    p.rect_filled(trilho, Rounding::same(2.0 * k), rgb(12, 12, 12));
    p.rect_stroke(trilho, Rounding::same(2.0 * k), Stroke::new(1.0_f32, Color32::from_white_alpha(120)));
    let frac = (ganho.clamp(-12.0, 12.0) + 12.0) / 24.0;
    let y = trilho.bottom() - frac * trilho.height();
    let capa = Rect::from_center_size(pos2(trilho.center().x, y), vec2(22.0 * k, 12.0 * k));
    p.rect_filled(capa.translate(vec2(1.5 * k, 2.0 * k)), Rounding::same(2.0 * k), Color32::from_black_alpha(70));
    grad_v(p, capa, &[(0.0, rgb(250, 250, 248)), (0.5, rgb(196, 195, 190)), (1.0, rgb(150, 149, 144))]);
    p.rect_stroke(capa, Rounding::same(2.0 * k), Stroke::new(1.0_f32, rgb(90, 90, 86)));
    p.line_segment(
        [pos2(capa.left() + 2.0 * k, y), pos2(capa.right() - 2.0 * k, y)],
        Stroke::new(1.5 * k, rgb(40, 40, 40)),
    );
    p.text(
        pos2(r.center().x, r.bottom() - 2.0 * k),
        Align2::CENTER_BOTTOM,
        rotulo,
        FontId::proportional(9.0 * k),
        rgb(40, 40, 40),
    );
}

/// Botão de função quadrado (aperta e trava) com lâmpada âmbar em cima.
pub fn botao_funcao(p: &Painter, r: Rect, rotulo: &str, ligado: bool) {
    let k = r.height() / 50.0;
    led(p, pos2(r.center().x, r.top() + 5.0 * k), 3.5 * k, rgb(255, 176, 50), ligado);
    let corpo = Rect::from_min_max(pos2(r.left(), r.top() + 13.0 * k), pos2(r.right(), r.bottom() - 13.0 * k));
    let desce = if ligado { 2.0 * k } else { 0.0 };
    p.rect_filled(corpo.translate(vec2(0.0, 3.0 * k)), Rounding::same(2.0 * k), rgb(92, 91, 87));
    let face = corpo.translate(vec2(0.0, desce));
    if ligado {
        grad_v(p, face, &[(0.0, rgb(180, 179, 174)), (1.0, rgb(212, 211, 206))]);
    } else {
        grad_v(p, face, &[(0.0, rgb(252, 252, 250)), (1.0, rgb(184, 183, 178))]);
    }
    p.rect_stroke(face, Rounding::same(2.0 * k), Stroke::new(1.0_f32, rgb(100, 99, 95)));
    p.text(
        pos2(r.center().x, r.bottom()),
        Align2::CENTER_BOTTOM,
        rotulo,
        FontId::proportional(8.0 * k),
        rgb(38, 38, 38),
    );
}

/// Botão giratório cromado recartilhado. `valor` em -1..+1 (-135°..+135°).
pub fn botao_giratorio(p: &Painter, c: Pos2, raio: f32, valor: f32, rotulo_esq: &str, rotulo_dir: &str) {
    // escala em volta
    for i in 0..=10 {
        let a = (-135.0 + 27.0 * i as f32).to_radians();
        let (s, co) = a.sin_cos();
        let (r1, r2) = (raio * 1.18, raio * if i % 5 == 0 { 1.36 } else { 1.27 });
        p.line_segment(
            [pos2(c.x + r1 * s, c.y - r1 * co), pos2(c.x + r2 * s, c.y - r2 * co)],
            Stroke::new(1.2_f32, rgb(48, 48, 48)),
        );
    }
    p.text(
        pos2(c.x - raio * 1.25, c.y + raio * 1.15),
        Align2::CENTER_TOP,
        rotulo_esq,
        FontId::proportional(raio * 0.42),
        rgb(40, 40, 40),
    );
    p.text(
        pos2(c.x + raio * 1.25, c.y + raio * 1.15),
        Align2::CENTER_TOP,
        rotulo_dir,
        FontId::proportional(raio * 0.42),
        rgb(40, 40, 40),
    );
    p.circle_filled(c + vec2(1.5, 2.5), raio, Color32::from_black_alpha(70));
    p.circle_filled(c, raio, rgb(70, 70, 70));
    // recartilhado
    for i in 0..48 {
        let a = i as f32 * PI / 24.0;
        let (s, co) = a.sin_cos();
        p.line_segment(
            [pos2(c.x + raio * 0.86 * s, c.y - raio * 0.86 * co), pos2(c.x + raio * s, c.y - raio * co)],
            Stroke::new(1.0_f32, if i % 2 == 0 { rgb(210, 210, 208) } else { rgb(110, 110, 108) }),
        );
    }
    let topo = Rect::from_center_size(c, vec2(raio * 1.7, raio * 1.7));
    grad_radial(
        &p.with_clip_rect(topo),
        topo,
        c + vec2(-raio * 0.3, -raio * 0.35),
        raio * 1.1,
        &[(0.0, rgb(255, 255, 255)), (0.55, rgb(196, 196, 194)), (1.0, rgb(98, 98, 96))],
    );
    p.circle_stroke(c, raio * 0.85, Stroke::new(1.0_f32, rgb(90, 90, 88)));
    let a = (valor.clamp(-1.0, 1.0) * 135.0).to_radians();
    let (s, co) = a.sin_cos();
    p.line_segment(
        [pos2(c.x + raio * 0.25 * s, c.y - raio * 0.25 * co), pos2(c.x + raio * 0.78 * s, c.y - raio * 0.78 * co)],
        Stroke::new(raio * 0.12, rgb(45, 45, 45)),
    );
}
