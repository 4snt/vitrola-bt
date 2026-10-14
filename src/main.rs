//! Vitrola BT: ouça e grave sua vitrola Bluetooth no Linux. O PC vira caixa
//! de som (A2DP Sink), recebe o SBC com o maior bitrate que o chip aceita e
//! mostra tudo num tape deck dos anos 70.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

mod desenho;
mod dsp;
mod estado;
mod flac;
mod medidor;
mod prefs;
mod sbc;
mod ui;

#[cfg(target_os = "linux")]
mod agente;
#[cfg(target_os = "linux")]
mod audio;
#[cfg(target_os = "linux")]
mod backend;
#[cfg(target_os = "linux")]
mod bandeja;
#[cfg(target_os = "linux")]
mod bluez;
#[cfg(target_os = "linux")]
mod instancia;
#[cfg(target_os = "linux")]
mod receptor;
#[cfg(unix)]
mod saida;

use std::sync::mpsc;
use std::time::Duration;

use estado::{Comando, Compartilhado, APP_ID};

const AJUDA: &str = "\
vitrola-bt — ouça e grave sua vitrola Bluetooth no Linux

Uso:
  vitrola-bt                 abre o deck (só uma janela por sessão)
  vitrola-bt --sem-interface receptor no terminal: liga o Bluetooth, toca e espera Ctrl+C
  vitrola-bt --version
";

fn icone() -> Option<egui::IconData> {
    let png = include_bytes!("../data/vitrola-bt-256.png");
    let dec = png::Decoder::new(&png[..]);
    let mut leitor = dec.read_info().ok()?;
    let mut buf = vec![0; leitor.output_buffer_size()];
    let info = leitor.next_frame(&mut buf).ok()?;
    (info.color_type == png::ColorType::Rgba).then(|| egui::IconData {
        rgba: buf[..info.buffer_size()].to_vec(),
        width: info.width,
        height: info.height,
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("vitrola-bt {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{AJUDA}");
        return;
    }
    let sem_interface = args.iter().any(|a| a == "--sem-interface" || a == "--headless");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("vitrola-bt")
        .enable_all()
        .build()
        .expect("runtime tokio");
    let c = Compartilhado::novo();
    let (tx_ui, rx_ui) = mpsc::channel();
    #[cfg(not(unix))]
    let _ = &tx_ui;
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Comando>();
    let prefs = prefs::Prefs::carregar();
    c.trava().som = prefs.som.clone();

    // Instância única (só para a janela; o modo terminal pode rodar à parte)
    #[cfg(target_os = "linux")]
    let sessao = if sem_interface {
        None
    } else {
        match rt.block_on(instancia::garantir(tx_ui.clone(), c.repintar.clone())) {
            instancia::Instancia::JaAberta => {
                eprintln!("O Vitrola BT já está aberto: trazendo a janela para a frente.");
                return;
            }
            instancia::Instancia::Primeira(conn) => Some(conn),
            instancia::Instancia::SemSessao => None,
        }
    };

    #[cfg(target_os = "linux")]
    rt.spawn(backend::rodar(c.clone(), rx, prefs.tocar_auto, sem_interface.then_some(true)));
    #[cfg(not(target_os = "linux"))]
    {
        c.trava().aviso = Some("Receptor Bluetooth ainda não disponível neste sistema (Linux por enquanto)".into());
        rt.spawn(async move {
            let mut rx = rx;
            while let Some(cmd) = rx.recv().await {
                if let Comando::Sair(ack) = cmd {
                    let _ = ack.send(());
                }
            }
        });
    }

    let sair = |tx: &tokio::sync::mpsc::UnboundedSender<Comando>| {
        let (ack, espera) = mpsc::channel();
        if tx.send(Comando::Sair(ack)).is_ok() {
            let _ = espera.recv_timeout(Duration::from_secs(6));
        }
    };

    if sem_interface {
        rt.block_on(async {
            #[cfg(unix)]
            {
                let mut term =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
        });
        eprintln!("Encerrando...");
        sair(&tx);
        rt.shutdown_timeout(Duration::from_secs(2));
        return;
    }

    // SIGTERM (desligar o PC, kill) fecha a janela pelo caminho normal: grava o
    // FLAC, solta a vitrola e deixa o PC invisível
    #[cfg(unix)]
    {
        let (tx_ui, rep) = (tx_ui.clone(), c.repintar.clone());
        rt.spawn(async move {
            if let Ok(mut term) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                term.recv().await;
                let _ = tx_ui.send(estado::EventoUi::Sair);
                rep.pedir();
            }
        });
    }

    #[cfg(target_os = "linux")]
    let tem_bandeja = match &sessao {
        Some(conn) => rt.block_on(bandeja::iniciar(conn, c.clone(), tx.clone(), tx_ui.clone())).is_ok(),
        None => false,
    };
    #[cfg(not(target_os = "linux"))]
    let tem_bandeja = false;

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Vitrola BT")
        .with_app_id(APP_ID)
        .with_inner_size([820.0, 826.0])
        .with_min_inner_size([560.0, 564.0]);
    if let Some(i) = icone() {
        viewport = viewport.with_icon(i);
    }
    let opcoes = eframe::NativeOptions { viewport, ..Default::default() };
    let (c2, tx2) = (c.clone(), tx.clone());
    let r = eframe::run_native(
        "Vitrola BT",
        opcoes,
        Box::new(move |cc| Ok(Box::new(ui::App::new(cc, c2, tx2, rx_ui, prefs, tem_bandeja)))),
    );
    if let Err(e) = r {
        eprintln!("Não consegui abrir a janela: {e}");
    }
    // on_exit já pediu para encerrar; garante caso a janela tenha falhado
    sair(&tx);
    #[cfg(target_os = "linux")]
    drop(sessao);
    rt.shutdown_timeout(Duration::from_secs(2));
}
