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

/// Alumínio escovado fino: veios longos e de pouco contraste (como o de um
/// aparelho de verdade fotografado), com leve tom quente.
pub fn textura_aluminio() -> ColorImage {
    let (w, h) = (2048usize, 512usize);
    let mut px = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        // cada linha tem um "veio": ruído suavizado ao longo de x (traços longos)
        let base = (ruido(0, y as u32, 11) - 0.5) * 7.0;
        let mut acumulado = 0.0f32;
        for x in 0..w {
            let alvo = (ruido((x / 23) as u32, y as u32, 7) - 0.5) * 5.0;
            acumulado += (alvo - acumulado) * 0.06;
            let fino = (ruido(x as u32, y as u32, 3) - 0.5) * 2.2;
            let v = (198.0 + base + acumulado + fino).clamp(0.0, 255.0);
            px.extend_from_slice(&[(v + 2.0) as u8, (v + 1.0) as u8, (v - 3.0).max(0.0) as u8, 255]);
        }
    }
    ColorImage::from_rgba_unmultiplied([w, h], &px)
}

/// Sombra difusa (luz de estúdio): camadas cada vez maiores e mais transparentes.
pub fn sombra_suave(p: &Painter, r: Rect, raio: f32, alcance: f32, alfa: u8) {
    let passos = 8;
    for i in (1..=passos).rev() {
        let t = i as f32 / passos as f32;
        let a = (alfa as f32 * (1.0 - t).powf(1.6) / passos as f32 * 2.2) as u8;
        p.rect_filled(
            r.expand(alcance * t).translate(vec2(0.0, alcance * t * 0.45)),
            Rounding::same(raio + alcance * t),
            Color32::from_black_alpha(a),
        );
    }
}

/// Disco com gradiente radial (do centro para a borda).
pub fn disco_radial(p: &Painter, c: Pos2, r: f32, centro: Color32, borda: Color32) {
    let n = 48u32;
    let mut m = Mesh::default();
    m.colored_vertex(c, centro);
    for i in 0..=n {
        let a = i as f32 / n as f32 * 2.0 * PI;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * r, borda);
        if i > 0 {
            m.add_triangle(0, i, i + 1);
        }
    }
    p.add(Shape::mesh(m));
}

/// Brilho de luz (LED, lâmpada): halo que some suavemente.
pub fn halo(p: &Painter, c: Pos2, r: f32, cor: Color32, intensidade: f32) {
    let centro = Color32::from_rgba_unmultiplied(cor.r(), cor.g(), cor.b(), (90.0 * intensidade) as u8);
    disco_radial(p, c, r, centro, Color32::from_rgba_unmultiplied(cor.r(), cor.g(), cor.b(), 0));
}

/// Reflexo diagonal discreto num vidro.
pub fn reflexo(p: &Painter, r: Rect, alfa: u8) {
    let pc = p.with_clip_rect(r);
    let mut m = Mesh::default();
    let (x0, y0, w, h) = (r.left(), r.top(), r.width(), r.height());
    let pts =
        [pos2(x0 + w * 0.05, y0), pos2(x0 + w * 0.42, y0), pos2(x0 + w * 0.12, y0 + h), pos2(x0 - w * 0.25, y0 + h)];
    let topo = Color32::from_white_alpha(alfa);
    let fundo = Color32::from_white_alpha(0);
    m.colored_vertex(pts[0], topo);
    m.colored_vertex(pts[1], topo);
    m.colored_vertex(pts[2], fundo);
    m.colored_vertex(pts[3], fundo);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    pc.add(Shape::mesh(m));
}

/// Texto serigrafado no metal: tinta fina com um fio de luz embaixo.
pub fn gravado(p: &Painter, pos: Pos2, align: Align2, txt: &str, fonte: FontId, cor: Color32) {
    p.text(pos + vec2(0.0, 0.8), align, txt, fonte.clone(), Color32::from_white_alpha(70));
    p.text(pos, align, txt, fonte, Color32::from_rgba_unmultiplied(cor.r(), cor.g(), cor.b(), 215));
}

pub fn parafuso(p: &Painter, c: Pos2, r: f32) {
    disco_radial(p, c + vec2(0.0, r * 0.3), r * 1.25, Color32::from_black_alpha(55), Color32::TRANSPARENT);
    disco_radial(p, c, r, rgb(150, 150, 148), rgb(96, 96, 94));
    p.circle_stroke(c, r, Stroke::new(0.6_f32, Color32::from_black_alpha(90)));
    // sextavado interno
    let hex: Vec<Pos2> =
        (0..6).map(|i| c + vec2((i as f32 * PI / 3.0).cos(), (i as f32 * PI / 3.0).sin()) * r * 0.45).collect();
    p.add(Shape::convex_polygon(hex, rgb(46, 46, 46), Stroke::NONE));
}

/// Janela de vidro preto fumê: moldura escura, sombra interna e reflexo.
pub fn vidro(p: &Painter, r: Rect, k: f32) -> Rect {
    sombra_suave(p, r, 3.0 * k, 3.0 * k, 90);
    p.rect_filled(r, Rounding::same(3.0 * k), rgb(22, 22, 24));
    p.rect_stroke(r, Rounding::same(3.0 * k), Stroke::new(1.0_f32, Color32::from_white_alpha(28)));
    let dentro = r.shrink(4.0 * k);
    p.rect_filled(dentro, Rounding::same(2.0 * k), rgb(7, 8, 9));
    // sombra interna: o vidro fica afundado na frente do aparelho
    for i in 0..6 {
        let a = (60 - i * 10) as u8;
        p.rect_stroke(
            dentro.shrink(i as f32 * k),
            Rounding::same(2.0 * k),
            Stroke::new(1.0 * k, Color32::from_black_alpha(a)),
        );
    }
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

/// VU retroiluminado: ponto quente no centro que esmaece nas bordas, escala
/// de VU e de modulação (%), ponteiro com sombra projetada e vidro na frente.
/// `valor` em VU.
pub fn vu(p: &Painter, r: Rect, valor: f32, canal: &str, tema: TemaVu) {
    let (paradas, tinta, vermelho) = tema.paleta();
    let k = r.width() / 214.0;
    p.rect_filled(r, Rounding::same(3.0 * k), rgb(10, 10, 11));
    let face = r.shrink(5.0 * k);
    let pc = p.with_clip_rect(face);
    grad_radial(&pc, face, pos2(face.center().x, face.bottom() + face.height() * 0.1), face.width() * 0.78, &paradas);
    // vinheta: a luz não chega igual nos cantos
    for i in 0..6 {
        let a = 46 - i * 7;
        pc.rect_stroke(
            face.shrink(i as f32 * 2.2 * k),
            Rounding::ZERO,
            Stroke::new(2.4 * k, Color32::from_black_alpha(a as u8)),
        );
    }
    let piv = pos2(face.center().x, face.bottom() + 6.0 * k);
    let raio = face.height() * 0.80;
    let ponto = |vu: f32, rr: f32| {
        let a = angulo_vu(vu);
        pos2(piv.x + rr * a.sin(), piv.y - rr * a.cos())
    };
    let arco =
        |de: f32, ate: f32, rr: f32| (0..=48).map(|i| ponto(de + (ate - de) * i as f32 / 48.0, rr)).collect::<Vec<_>>();
    pc.add(Shape::line(arco(-20.0, 0.0, raio), Stroke::new(0.9 * k, tinta)));
    pc.add(Shape::line(arco(0.0, 3.0, raio), Stroke::new(0.9 * k, vermelho)));
    pc.add(Shape::line(arco(0.0, 3.0, raio + 3.0 * k), Stroke::new(3.6 * k, vermelho.gamma_multiply(0.9))));
    let fonte = FontId::proportional(8.6 * k);
    for vu in [-20.0f32, -10.0, -7.0, -5.0, -3.0, -2.0, -1.0, 0.0, 1.0, 2.0, 3.0] {
        let cor = if vu > 0.0 { vermelho } else { tinta };
        let longo = [-20.0, -10.0, -5.0, -3.0, 0.0, 3.0].contains(&vu);
        pc.line_segment(
            [ponto(vu, raio), ponto(vu, raio + if longo { 8.0 } else { 5.0 } * k)],
            Stroke::new(0.9 * k, cor),
        );
        if longo {
            let txt = if vu == 0.0 { "0".to_string() } else { format!("{:+}", vu as i32) };
            pc.text(ponto(vu, raio + 15.5 * k), Align2::CENTER_CENTER, txt, fonte.clone(), cor);
        }
    }
    // escala de modulação (%) por dentro, como nos medidores reais
    pc.add(Shape::line(arco(-14.0, 0.0, raio * 0.86), Stroke::new(0.6 * k, tinta.gamma_multiply(0.7))));
    for pct in [20.0f32, 40.0, 60.0, 80.0, 100.0] {
        let vu = 20.0 * (pct / 100.0).log10();
        pc.line_segment(
            [ponto(vu, raio * 0.86), ponto(vu, raio * 0.83)],
            Stroke::new(0.6 * k, tinta.gamma_multiply(0.7)),
        );
        pc.text(
            ponto(vu, raio * 0.77),
            Align2::CENTER_CENTER,
            format!("{pct:.0}"),
            FontId::proportional(6.0 * k),
            tinta.gamma_multiply(0.75),
        );
    }
    pc.text(pos2(piv.x, piv.y - raio * 0.40), Align2::CENTER_CENTER, "VU", FontId::proportional(13.0 * k), tinta);
    pc.text(
        pos2(face.left() + 9.0 * k, face.bottom() - 9.0 * k),
        Align2::LEFT_BOTTOM,
        canal,
        FontId::proportional(8.5 * k),
        tinta.gamma_multiply(0.85),
    );
    // ponteiro: sombra projetada no mostrador e agulha fina
    let ponta = ponto(valor, raio + 9.0 * k);
    for j in 1..=4 {
        let d = vec2(1.4 * k * j as f32, 1.8 * k * j as f32);
        pc.line_segment([piv + d, ponta + d], Stroke::new(1.3 * k, Color32::from_black_alpha(22)));
    }
    let agulha = if tema == TemaVu::Azul { rgb(245, 245, 240) } else { rgb(20, 16, 12) };
    pc.line_segment([piv, ponto(valor, raio * 0.55)], Stroke::new(1.6 * k, agulha));
    pc.line_segment([ponto(valor, raio * 0.55), ponta], Stroke::new(1.0 * k, agulha));
    // tampa do pivô (preta, com um fio de luz)
    disco_radial(&pc, piv, 13.0 * k, rgb(38, 38, 40), rgb(8, 8, 9));
    pc.circle_stroke(piv, 13.0 * k, Stroke::new(0.8 * k, Color32::from_white_alpha(30)));
    reflexo(&pc, face, 30);
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
    p.rect_filled(r, Rounding::same(3.0 * k), rgb(5, 6, 7));
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
                rgb(255, 64, 48)
            } else if s >= SEGMENTOS - 5 {
                rgb(255, 178, 44)
            } else {
                rgb(84, 230, 120)
            };
            let seg = Rect::from_min_size(pos2(x, y), vec2(larg - 6.0 * k, alt - 2.2 * k));
            let aceso = s < acesos || (pico > 0 && s + 1 == pico);
            if aceso {
                // brilho difuso do segmento aceso
                p.rect_filled(seg.expand(1.6 * k), Rounding::same(2.0 * k), cor.gamma_multiply(0.22));
                p.rect_filled(seg, Rounding::same(0.8 * k), cor);
                p.rect_filled(
                    Rect::from_min_size(seg.min, vec2(seg.width(), seg.height() * 0.4)),
                    Rounding::same(0.8 * k),
                    Color32::from_white_alpha(46),
                );
            } else {
                p.rect_filled(seg, Rounding::same(0.8 * k), mix(cor, rgb(8, 8, 9), 0.9));
            }
        }
        p.text(
            pos2(x + (larg - 6.0 * k) / 2.0, r.bottom() - 4.0 * k),
            Align2::CENTER_BOTTOM,
            rotulos[i],
            FontId::proportional(8.4 * k),
            Color32::from_white_alpha(120),
        );
    }
    reflexo(p, r, 14);
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
    let prof = 7.0 * k;
    let alt_face = r.height() - prof;
    let desce = if afundada { prof - 2.0 * k } else { 0.0 };
    // nicho escuro do teclado e espessura da tecla
    p.rect_filled(
        Rect::from_min_max(pos2(r.left() - 1.5 * k, r.top() + 4.0 * k), r.max + vec2(1.5 * k, 1.5 * k)),
        Rounding::same(3.0 * k),
        rgb(34, 34, 35),
    );
    let face = Rect::from_min_size(pos2(r.left(), r.top() + desce), vec2(r.width(), alt_face));
    sombra_suave(p, face, 2.0 * k, if afundada { 1.5 } else { prof * 0.9 } * k / k * k, 130);
    p.rect_filled(
        Rect::from_min_max(
            pos2(face.left(), face.bottom() - 2.0 * k),
            pos2(face.right(), face.bottom() + prof - desce),
        ),
        Rounding::same(2.0 * k),
        rgb(132, 131, 127),
    );
    if afundada {
        grad_v(p, face, &[(0.0, rgb(186, 185, 181)), (1.0, rgb(204, 203, 199))]);
    } else {
        grad_v(p, face, &[(0.0, rgb(226, 225, 221)), (0.6, rgb(210, 209, 205)), (1.0, rgb(190, 189, 185))]);
    }
    let pf = p.with_clip_rect(face);
    let mut y = face.top() + 1.0;
    let mut n = 0;
    while y < face.bottom() {
        let c = if n % 2 == 0 { Color32::from_white_alpha(14) } else { Color32::from_black_alpha(7) };
        pf.line_segment([pos2(face.left(), y), pos2(face.right(), y)], Stroke::new(0.7_f32, c));
        y += 1.6;
        n += 1;
    }
    p.rect_stroke(face, Rounding::same(2.0 * k), Stroke::new(0.8_f32, Color32::from_black_alpha(90)));
    if !afundada {
        p.line_segment(
            [pos2(face.left() + 2.0 * k, face.top() + 0.8), pos2(face.right() - 2.0 * k, face.top() + 0.8)],
            Stroke::new(0.8_f32, Color32::from_white_alpha(110)),
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

/// LED em cúpula: aro escuro, lente com gradiente, reflexo e halo difuso aceso.
pub fn led(p: &Painter, c: Pos2, r: f32, cor: Color32, aceso: bool) {
    if aceso {
        halo(p, c, r * 4.2, cor, 1.0);
    }
    disco_radial(p, c, r * 1.3, rgb(64, 64, 64), rgb(24, 24, 24));
    let base = if aceso { cor } else { mix(cor, rgb(16, 13, 11), 0.84) };
    let centro = if aceso { mix(cor, Color32::WHITE, 0.55) } else { mix(base, Color32::WHITE, 0.06) };
    disco_radial(p, c, r, centro, mix(base, Color32::BLACK, 0.4));
    disco_radial(
        p,
        c + vec2(-r * 0.32, -r * 0.36),
        r * 0.38,
        Color32::from_white_alpha(if aceso { 170 } else { 60 }),
        Color32::from_white_alpha(0),
    );
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

/// Display fluorescente (VFD) verde-azulado atrás de vidro fumê, com trama de fósforo.
pub fn display(p: &Painter, r: Rect, linhas: &[(&str, String)]) {
    let k = r.height() / 190.0;
    sombra_suave(p, r, 3.0 * k, 3.0 * k, 80);
    p.rect_filled(r, Rounding::same(3.0 * k), rgb(20, 21, 22));
    let tela = r.shrink(3.0 * k);
    p.rect_filled(tela, Rounding::same(2.0 * k), rgb(3, 8, 7));
    let fonte = FontId::monospace(12.5 * k);
    let alt = (tela.height() - 14.0 * k) / linhas.len().max(1) as f32;
    for (i, (chave, valor)) in linhas.iter().enumerate() {
        let y = tela.top() + 7.0 * k + i as f32 * alt + alt / 2.0;
        p.text(pos2(tela.left() + 10.0 * k, y), Align2::LEFT_CENTER, *chave, fonte.clone(), rgb(40, 128, 106));
        let v = caber(p, valor, &fonte, tela.width() - 136.0 * k);
        for d in [vec2(0.0, 0.0), vec2(0.6, 0.0), vec2(0.0, 0.6)] {
            p.text(
                pos2(tela.left() + 126.0 * k, y) + d,
                Align2::LEFT_CENTER,
                &v,
                fonte.clone(),
                Color32::from_rgba_unmultiplied(125, 242, 207, 26),
            );
        }
        p.text(pos2(tela.left() + 126.0 * k, y), Align2::LEFT_CENTER, v, fonte.clone(), rgb(132, 246, 212));
    }
    // trama de fósforo e vidro fumê por cima
    let mut y = tela.top();
    while y < tela.bottom() {
        p.line_segment(
            [pos2(tela.left(), y), pos2(tela.right(), y)],
            Stroke::new(0.6_f32, Color32::from_black_alpha(40)),
        );
        y += 2.2 * k.max(0.5);
    }
    reflexo(p, tela, 18);
    p.rect_stroke(tela, Rounding::same(2.0 * k), Stroke::new(1.0_f32, Color32::from_black_alpha(160)));
}

// ---------- equalizador gráfico e funções ----------

/// Cursor deslizante de equalizador: fenda escura, escala fina e capa de
/// alumínio com ranhuras, com sombra difusa. `ganho` em dB (-12..+12).
pub fn cursor_eq(p: &Painter, r: Rect, ganho: f32, rotulo: &str) {
    let k = r.height() / 120.0;
    let trilho = Rect::from_center_size(pos2(r.center().x, r.top() + 54.0 * k), vec2(3.4 * k, 92.0 * k));
    for (i, db) in [12.0f32, 6.0, 0.0, -6.0, -12.0].iter().enumerate() {
        let y = trilho.top() + trilho.height() * i as f32 / 4.0;
        let largura = if *db == 0.0 { 9.0 } else { 5.0 } * k;
        let tinta = Color32::from_black_alpha(150);
        p.line_segment(
            [pos2(trilho.left() - largura - 3.0 * k, y), pos2(trilho.left() - 3.0 * k, y)],
            Stroke::new(0.8_f32, tinta),
        );
        p.line_segment(
            [pos2(trilho.right() + 3.0 * k, y), pos2(trilho.right() + largura + 3.0 * k, y)],
            Stroke::new(0.8_f32, tinta),
        );
    }
    p.rect_filled(trilho.expand(1.0 * k), Rounding::same(2.0 * k), Color32::from_white_alpha(40));
    p.rect_filled(trilho, Rounding::same(1.6 * k), rgb(10, 10, 10));
    let frac = (ganho.clamp(-12.0, 12.0) + 12.0) / 24.0;
    let y = trilho.bottom() - frac * trilho.height();
    let capa = Rect::from_center_size(pos2(trilho.center().x, y), vec2(22.0 * k, 13.0 * k));
    sombra_suave(p, capa, 2.0 * k, 3.0 * k, 140);
    grad_v(p, capa, &[(0.0, rgb(232, 232, 229)), (0.5, rgb(204, 203, 199)), (1.0, rgb(160, 159, 155))]);
    for d in [-3.0f32, 0.0, 3.0] {
        let yy = y + d * k;
        p.line_segment(
            [pos2(capa.left() + 2.0 * k, yy), pos2(capa.right() - 2.0 * k, yy)],
            Stroke::new(0.8_f32, Color32::from_black_alpha(if d == 0.0 { 150 } else { 60 })),
        );
        p.line_segment(
            [pos2(capa.left() + 2.0 * k, yy + 0.8), pos2(capa.right() - 2.0 * k, yy + 0.8)],
            Stroke::new(0.6_f32, Color32::from_white_alpha(70)),
        );
    }
    p.rect_stroke(capa, Rounding::same(1.8 * k), Stroke::new(0.8_f32, Color32::from_black_alpha(110)));
    gravado(
        p,
        pos2(r.center().x, r.bottom() - 2.0 * k),
        Align2::CENTER_BOTTOM,
        rotulo,
        FontId::proportional(8.4 * k),
        rgb(38, 38, 38),
    );
}

/// Botão de função em alumínio escovado (aperta e trava), com LED em cima.
pub fn botao_funcao(p: &Painter, r: Rect, rotulo: &str, ligado: bool) {
    let k = r.height() / 50.0;
    led(p, pos2(r.center().x, r.top() + 5.0 * k), 3.0 * k, rgb(255, 168, 56), ligado);
    let corpo = Rect::from_min_max(pos2(r.left(), r.top() + 13.0 * k), pos2(r.right(), r.bottom() - 13.0 * k));
    let desce = if ligado { 1.4 * k } else { 0.0 };
    // nicho escuro onde o botão afunda
    p.rect_filled(corpo.expand(1.2 * k), Rounding::same(2.4 * k), rgb(40, 40, 41));
    sombra_suave(p, corpo.translate(vec2(0.0, desce)), 2.0 * k, if ligado { 1.0 } else { 3.0 } * k, 110);
    let face = corpo.translate(vec2(0.0, desce));
    if ligado {
        grad_v(p, face, &[(0.0, rgb(188, 187, 183)), (1.0, rgb(206, 205, 201))]);
    } else {
        grad_v(p, face, &[(0.0, rgb(230, 229, 225)), (0.55, rgb(212, 211, 207)), (1.0, rgb(186, 185, 181))]);
    }
    let pf = p.with_clip_rect(face);
    let mut y = face.top() + 1.0;
    let mut i = 0;
    while y < face.bottom() {
        let c = if i % 2 == 0 { Color32::from_white_alpha(16) } else { Color32::from_black_alpha(8) };
        pf.line_segment([pos2(face.left(), y), pos2(face.right(), y)], Stroke::new(0.7_f32, c));
        y += 1.6;
        i += 1;
    }
    p.rect_stroke(face, Rounding::same(1.8 * k), Stroke::new(0.8_f32, Color32::from_black_alpha(95)));
    if !ligado {
        p.line_segment(
            [pos2(face.left() + 2.0 * k, face.top() + 0.8), pos2(face.right() - 2.0 * k, face.top() + 0.8)],
            Stroke::new(0.8_f32, Color32::from_white_alpha(120)),
        );
    }
    gravado(
        p,
        pos2(r.center().x, r.bottom()),
        Align2::CENTER_BOTTOM,
        rotulo,
        FontId::proportional(7.6 * k),
        rgb(36, 36, 36),
    );
}

/// Botão giratório de alumínio torneado: saia recartilhada, tampa com o
/// reflexo em leque do metal torneado, marcas de usinagem e sombra difusa.
/// `valor` em -1..+1 (-135°..+135°).
pub fn botao_giratorio(p: &Painter, c: Pos2, raio: f32, valor: f32, rotulo_esq: &str, rotulo_dir: &str) {
    for i in 0..=10 {
        let a = (-135.0 + 27.0 * i as f32).to_radians();
        let (s, co) = a.sin_cos();
        let (r1, r2) = (raio * 1.17, raio * if i % 5 == 0 { 1.33 } else { 1.25 });
        p.line_segment(
            [pos2(c.x + r1 * s, c.y - r1 * co), pos2(c.x + r2 * s, c.y - r2 * co)],
            Stroke::new(1.0_f32, Color32::from_black_alpha(170)),
        );
    }
    let tinta = rgb(40, 40, 40);
    gravado(
        p,
        pos2(c.x - raio * 1.25, c.y + raio * 1.12),
        Align2::CENTER_TOP,
        rotulo_esq,
        FontId::proportional(raio * 0.36),
        tinta,
    );
    gravado(
        p,
        pos2(c.x + raio * 1.25, c.y + raio * 1.12),
        Align2::CENTER_TOP,
        rotulo_dir,
        FontId::proportional(raio * 0.36),
        tinta,
    );
    // sombra difusa no painel
    disco_radial(
        p,
        c + vec2(raio * 0.08, raio * 0.22),
        raio * 1.42,
        Color32::from_black_alpha(120),
        Color32::TRANSPARENT,
    );
    // saia recartilhada
    disco_radial(p, c, raio, rgb(62, 62, 62), rgb(28, 28, 28));
    for i in 0..60 {
        let a = i as f32 * PI / 30.0;
        let (s, co) = a.sin_cos();
        p.line_segment(
            [pos2(c.x + raio * 0.84 * s, c.y - raio * 0.84 * co), pos2(c.x + raio * 0.99 * s, c.y - raio * 0.99 * co)],
            Stroke::new(
                0.8_f32,
                if i % 2 == 0 { Color32::from_white_alpha(46) } else { Color32::from_black_alpha(60) },
            ),
        );
    }
    // tampa torneada: reflexo anisotrópico (o "leque" de luz do metal torneado)
    let topo = raio * 0.80;
    let luz = -2.3f32; // direção da luz (canto superior esquerdo)
    let n = 96u32;
    let mut m = Mesh::default();
    m.colored_vertex(c, rgb(206, 206, 203));
    for i in 0..=n {
        let a = i as f32 / n as f32 * 2.0 * PI;
        let f = 0.5 + 0.5 * (2.0 * (a - luz)).cos();
        let v = (150.0 + 95.0 * f.powf(2.5)) as u8;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * topo, rgb(v, v, v.saturating_sub(3)));
        if i > 0 {
            m.add_triangle(0, i, i + 1);
        }
    }
    p.add(Shape::mesh(m));
    for j in 1..6 {
        p.circle_stroke(c, topo * j as f32 / 6.0, Stroke::new(0.5_f32, Color32::from_black_alpha(10)));
    }
    p.circle_stroke(c, topo, Stroke::new(1.0_f32, Color32::from_white_alpha(110)));
    p.circle_stroke(c, topo + 1.0, Stroke::new(0.8_f32, Color32::from_black_alpha(90)));
    // marca de posição gravada
    let a = (valor.clamp(-1.0, 1.0) * 135.0).to_radians();
    let (s, co) = a.sin_cos();
    let (q1, q2) =
        (pos2(c.x + topo * 0.40 * s, c.y - topo * 0.40 * co), pos2(c.x + topo * 0.88 * s, c.y - topo * 0.88 * co));
    p.line_segment(
        [q1 + vec2(0.6, 0.6), q2 + vec2(0.6, 0.6)],
        Stroke::new(raio * 0.08, Color32::from_white_alpha(120)),
    );
    p.line_segment([q1, q2], Stroke::new(raio * 0.08, rgb(48, 48, 48)));
}
