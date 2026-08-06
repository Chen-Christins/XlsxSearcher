#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alias;
mod app_state;
mod db;
mod scanner;
mod server;
mod types;

use app_state::AppState;
use std::sync::mpsc;

// Preferred local web port. Falls back to a free ephemeral port only when the
// preferred port is already taken, so the address stays stable across launches.
const PREFERRED_PORT: u16 = 8123;

fn main() {
    let state = AppState::new();
    let server_state = state.clone();
    let (port_tx, port_rx) = mpsc::channel::<u16>();

    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new().expect("create tokio runtime");
        runtime.block_on(async move {
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PREFERRED_PORT)).await
            {
                Ok(l) => l,
                Err(_) => tokio::net::TcpListener::bind("127.0.0.1:0")
                    .await
                    .expect("bind local server"),
            };
            let port = listener.local_addr().expect("local address").port();
            let _ = port_tx.send(port);
            axum::serve(listener, server::build_router(server_state))
                .await
                .expect("serve local api");
        });
    });

    let port = port_rx.recv().expect("receive server port");
    let web_token = state.web_token.clone();
    let url = format!("http://127.0.0.1:{}/?token={}", port, web_token);
    println!("WebUI 已启动: {}", url);

    let setup_state = state.clone();
    tauri::Builder::default()
        .manage(state)
        .setup(move |app| {
            setup_state.set_app_handle(app.handle().clone());
            let builder = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url.parse().expect("parse webui url")),
            )
            .title("XlsxSearcher")
            .inner_size(1280.0, 860.0)
            .min_inner_size(1000.0, 700.0)
            .user_agent(&server::webview_user_agent());

            // macOS keeps native traffic lights (red/yellow/green) overlaid on
            // the content; other platforms use a fully frameless window with
            // the custom titlebar controls drawn by the UI.
            #[cfg(target_os = "macos")]
            let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay);

            #[cfg(not(target_os = "macos"))]
            let builder = builder.decorations(false);

            builder.build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
