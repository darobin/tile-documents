mod car;

use car::{authority_from_path, parse_tile, Masl, TileContent};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::menu::{MenuBuilder, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Listener, Manager, State};
use tauri_plugin_window_state::{Builder as WindowStateBuilder, StateFlags, WindowExt};

// ── Shared state ─────────────────────────────────────────────────────────────

/// Authority string → loaded tile content.
struct TileStore(Mutex<HashMap<String, TileContent>>);

// ── Frontend-facing types ────────────────────────────────────────────────────

/// Sent to the frontend when a tile is opened (via command or file-open event).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TileOpenedPayload {
    pub authority: String,
    pub masl: Masl,
}

// ── Commands ─────────────────────────────────────────────────────────────────

/// Open a `.tile` file at the given path, load it into the store, and return
/// the tile info. The frontend should then navigate to `tile://<authority>/`.
#[tauri::command]
fn open_tile(
    path: String,
    state: State<'_, TileStore>,
    app: AppHandle,
) -> Result<TileOpenedPayload, String> {
    let p = PathBuf::from(&path);
    load_tile(&p, &state, &app).map_err(|e| e.to_string())
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn load_tile(
    path: &Path,
    state: &State<'_, TileStore>,
    app: &AppHandle,
) -> anyhow::Result<TileOpenedPayload> {
    let content = parse_tile(path)?;
    let authority = authority_from_path(path);
    let payload = TileOpenedPayload {
        authority: authority.clone(),
        masl: content.masl.clone(),
    };
    state.0.lock().unwrap().insert(authority, content);
    app.emit("tile:opened", &payload)?;
    Ok(payload)
}

// ── tile: custom protocol ─────────────────────────────────────────────────────

fn handle_tile_protocol(
    app: &AppHandle<impl tauri::Runtime>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let uri = request.uri();

    let authority = uri.host().unwrap_or("");
    // Normalise path: ensure it starts with '/'.
    let raw_path = uri.path();
    let path = if raw_path.is_empty() { "/" } else { raw_path };

    let store = app.state::<TileStore>();
    let guard = store.0.lock().unwrap();

    let error = |status: u16, msg: &str| {
        tauri::http::Response::builder()
            .status(status)
            .header("content-type", "text/plain")
            .body(msg.as_bytes().to_vec())
            .unwrap()
    };

    let tile = match guard.get(authority) {
        Some(t) => t,
        None => return error(404, "tile not loaded"),
    };

    // Walk the MASL resource map. Try the exact path first, then with/without
    // trailing slash, then "/index.html" fallback for the root.
    let candidates: &[&str] = &[
        path,
        if path.ends_with('/') { path.trim_end_matches('/') } else { path },
        if !path.ends_with('/') { &format!("{path}/") } else { path },
        if path == "/" { "/index.html" } else { path },
    ];

    let resource = candidates.iter().find_map(|p| tile.masl.resources.get(*p));

    let resource = match resource {
        Some(r) => r,
        None => return error(404, &format!("no resource at {path}")),
    };

    let src = match resource.get("src") {
        Some(s) => s.as_str(),
        None => return error(500, "resource missing src"),
    };
    let data = match tile.read_block(src) {
        Ok(d) => d,
        Err(e) => return error(500, &e.to_string()),
    };

    let content_type = resource
        .get("content-type")
        .cloned()
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let mut builder = tauri::http::Response::builder()
        .status(200)
        .header("content-type", &content_type)
        .header("access-control-allow-origin", "*");

    // Forward any other headers from the MASL resource entry.
    for (k, v) in resource {
        if k != "content-type" && k != "src" {
            builder = builder.header(k.as_str(), v.as_str());
        }
    }

    builder.body(data).unwrap()
}

// ── App entry point ───────────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(WindowStateBuilder::new().build())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(TileStore(Mutex::new(HashMap::new())))
        .register_uri_scheme_protocol("tile", |ctx, request| {
            handle_tile_protocol(ctx.app_handle(), request)
        })
        .invoke_handler(tauri::generate_handler![open_tile])
        .menu(|app| {
            let accel = if cfg!(target_os = "macos") {
                "Command+Control+F"
            } else {
                "F11"
            };

            let toggle_fs =
                MenuItemBuilder::with_id("toggle_fullscreen", "Toggle Full Screen")
                    .accelerator(accel)
                    .build(app)?;

            let view = SubmenuBuilder::new(app, "View").item(&toggle_fs).build()?;

            let mut builder = MenuBuilder::new(app);

            #[cfg(target_os = "macos")]
            {
                use tauri::menu::PredefinedMenuItem;
                let app_menu = SubmenuBuilder::new(app, "Tile Documents")
                    .item(&PredefinedMenuItem::about(app, None, None)?)
                    .separator()
                    .item(&PredefinedMenuItem::services(app, None)?)
                    .separator()
                    .item(&PredefinedMenuItem::hide(app, None)?)
                    .item(&PredefinedMenuItem::hide_others(app, None)?)
                    .item(&PredefinedMenuItem::show_all(app, None)?)
                    .separator()
                    .item(&PredefinedMenuItem::quit(app, None)?)
                    .build()?;
                builder = builder.item(&app_menu);
            }

            builder.item(&view).build()
        })
        .on_menu_event(|app, event| {
            if event.id() == "toggle_fullscreen" {
                if let Some(window) = app.get_webview_window("main") {
                    let is_fs = window.is_fullscreen().unwrap_or(false);
                    let _ = window.set_fullscreen(!is_fs);
                }
            }
        })
        .setup(|app| {
            let app_handle = app.handle().clone();

            // Restore saved window state (position, size, fullscreen) and set
            // up a listener that notifies the frontend on any fullscreen change.
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.restore_state(StateFlags::all());

                // Track the last known fullscreen state so we emit only on
                // actual changes. This covers menu/keyboard, the macOS green
                // button, and ESC-to-exit – anything that causes a Resized
                // event.
                let app_for_event = app_handle.clone();
                let last_fs = Arc::new(AtomicBool::new(
                    window.is_fullscreen().unwrap_or(false),
                ));
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Resized(_)) {
                        if let Some(win) = app_for_event.get_webview_window("main") {
                            let is_fs = win.is_fullscreen().unwrap_or(false);
                            let prev = last_fs.swap(is_fs, Ordering::Relaxed);
                            if prev != is_fs {
                                let _ = app_for_event.emit("tile:fullscreen-changed", is_fs);
                            }
                        }
                    }
                });
            }

            // Handle files passed as CLI arguments (Windows / Linux).
            let args: Vec<String> = std::env::args().skip(1).collect();
            let state = app_handle.state::<TileStore>();
            for arg in &args {
                let p = PathBuf::from(arg);
                if p.extension().and_then(|e| e.to_str()) == Some("tile") && p.exists() {
                    let _ = load_tile(&p, &state, &app_handle);
                }
            }

            // Handle macOS / deep-link file-open events.
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            {
                let app_handle2 = app_handle.clone();
                app.listen("deep-link://new-url", move |event| {
                    if let Ok(urls) = serde_json::from_str::<Vec<String>>(event.payload()) {
                        let state = app_handle2.state::<TileStore>();
                        for url in urls {
                            if let Some(file_path) = url.strip_prefix("file://") {
                                let p = PathBuf::from(file_path);
                                let _ = load_tile(&p, &state, &app_handle2);
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running Tile Documents");
}
