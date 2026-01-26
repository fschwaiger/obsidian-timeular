use anyhow::Result;
use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, timeout};
use tray_icon::{
    menu::{Menu, MenuItem},
    TrayIconBuilder,
};

fn tracker_tcp_port() -> u16 {
    env::var("TRACKER_TCP_PORT")
        .expect("Environment variable TRACKER_TCP_PORT must be set.")
        .parse()
        .expect("TRACKER_TCP_PORT must be a valid u16 integer.")
}

fn tracker_name_prefix() -> String {
    env::var("TRACKER_NAME_PREFIX").expect("Environment variable TRACKER_NAME_PREFIX must be set.")
}

fn tracker_battery_uuid() -> String {
    env::var("TRACKER_BATTERY_UUID")
        .expect("Environment variable TRACKER_BATTERY_UUID must be set.")
}

fn tracker_indicator_uuid() -> String {
    env::var("TRACKER_INDICATOR_UUID")
        .expect("Environment variable TRACKER_INDICATOR_UUID must be set.")
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct DeviceState {
    battery: u8,
    indicator: u8,
}

struct AppState {
    connected_clients: Arc<RwLock<usize>>,
}

#[tokio::main]
async fn main() {
    println!("Tracker Companion starting...");

    // Initialize GTK for tray icon
    gtk::init().expect("Failed to initialize GTK");

    let app_state = Arc::new(AppState {
        connected_clients: Arc::new(RwLock::new(0)),
    });

    // Setup system tray
    let tray_menu = Menu::new();
    let quit_item = MenuItem::new("Quit", true, None);
    tray_menu.append(&quit_item).ok();

    let icon_data = create_tray_icon();
    let _tray_icon = TrayIconBuilder::new()
        .with_menu(Box::new(tray_menu))
        .with_icon(tray_icon::Icon::from_rgba(icon_data, 32, 32).expect("Failed to create icon"))
        .with_tooltip("Tracker Companion")
        .build()
        .expect("Failed to create tray icon");

    println!("System tray initialized");
    println!("Starting TCP server on port {}", tracker_tcp_port());

    // Start TCP server
    if let Err(e) = run_tcp_server(app_state).await {
        eprintln!("TCP server error: {:?}", e);
    }
}

fn create_tray_icon() -> Vec<u8> {
    let mut icon_data = vec![0u8; 32 * 32 * 4];
    for i in (0..icon_data.len()).step_by(4) {
        icon_data[i] = 50; // R
        icon_data[i + 1] = 150; // G
        icon_data[i + 2] = 250; // B
        icon_data[i + 3] = 255; // A
    }
    icon_data
}

async fn run_tcp_server(app_state: Arc<AppState>) -> Result<()> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", tracker_tcp_port())).await?;
    println!("Listening on 127.0.0.1:{}", tracker_tcp_port());

    loop {
        let (socket, addr) = listener.accept().await?;
        println!("Client connected from: {}", addr);

        let app_state = app_state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client(socket, app_state).await {
                eprintln!("Client handler error: {:?}", e);
            }
            println!("Client {} disconnected", addr);
        });
    }
}

async fn handle_client(socket: TcpStream, app_state: Arc<AppState>) -> Result<()> {
    *app_state.connected_clients.write().await += 1;
    let socket = Arc::new(Mutex::new(socket));

    // Start scanning for device
    println!("Starting Bluetooth scan...");

    match timeout(Duration::from_secs(10), find_and_connect_device()).await {
        Ok(Ok((peripheral, adapter))) => {
            println!("Device connected successfully");

            // Monitor device and send updates
            if let Err(e) = monitor_device(peripheral, adapter, socket.clone()).await {
                eprintln!("Monitor error: {:?}", e);
            }
        }
        Ok(Err(e)) => {
            eprintln!("Failed to connect to device: {:?}", e);
            let mut sock = socket.lock().await;
            sock.write_all(b"{\"error\":\"Device not found\"}\n")
                .await
                .ok();
        }
        Err(_) => {
            println!("Timeout: Device not found within 10 seconds");
            let mut sock = socket.lock().await;
            sock.write_all(b"{\"error\":\"Timeout - device not found\"}\n")
                .await
                .ok();
        }
    }

    *app_state.connected_clients.write().await -= 1;
    Ok(())
}

async fn find_and_connect_device() -> Result<(Peripheral, Adapter)> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    let central = adapters
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("No Bluetooth adapter found"))?;

    central.start_scan(ScanFilter::default()).await?;

    // Poll for device for up to 10 seconds
    for _ in 0..50 {
        sleep(Duration::from_millis(200)).await;

        let peripherals = central.peripherals().await?;
        for p in peripherals {
            if let Some(props) = p.properties().await? {
                if let Some(name) = props.local_name {
                    if name.starts_with(&tracker_name_prefix()) {
                        println!("Found tracker: {}", name);
                        central.stop_scan().await?;
                        p.connect().await?;
                        p.discover_services().await?;
                        return Ok((p, central));
                    }
                }
            }
        }
    }

    central.stop_scan().await?;
    anyhow::bail!("Tracker device not found")
}

async fn monitor_device(
    peripheral: Peripheral,
    adapter: Adapter,
    socket: Arc<Mutex<TcpStream>>,
) -> Result<()> {
    let mut last_battery: Option<u8> = None;
    let mut last_indicator: Option<u8> = None;

    // Subscribe to characteristics
    let services = peripheral.services();
    let battery_uuid = tracker_battery_uuid();
    let indicator_uuid = tracker_indicator_uuid();

    for service in &services {
        for char in &service.characteristics {
            let uuid_str = char.uuid.to_string();

            // Subscribe to battery and indicator characteristics
            if uuid_str == battery_uuid || uuid_str == indicator_uuid {
                if char.properties.contains(CharPropFlags::NOTIFY)
                    || char.properties.contains(CharPropFlags::INDICATE)
                {
                    peripheral.subscribe(char).await?;
                    println!("Subscribed to {}", uuid_str);

                    // Read initial value
                    if char.properties.contains(CharPropFlags::READ) {
                        if let Ok(data) = peripheral.read(char).await {
                            if !data.is_empty() {
                                if uuid_str == battery_uuid {
                                    last_battery = Some(data[0]);
                                } else if uuid_str == indicator_uuid {
                                    last_indicator = Some(data[0]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Send initial state if we have values
    if let (Some(battery), Some(indicator)) = (last_battery, last_indicator) {
        let state = DeviceState { battery, indicator };
        send_state(&socket, &state).await?;
    }

    // Listen for notifications
    let mut notification_stream = peripheral.notifications().await?;

    loop {
        tokio::select! {
            notification = notification_stream.next() => {
                if let Some(data) = notification {
                    let uuid_str = data.uuid.to_string();

                    if !data.value.is_empty() {
                        let value = data.value[0];
                        let mut changed = false;

                        if uuid_str == battery_uuid && last_battery != Some(value) {
                            println!("Battery changed: {}%", value);
                            last_battery = Some(value);
                            changed = true;
                        } else if uuid_str == indicator_uuid && last_indicator != Some(value) {
                            println!("Indicator changed: {}", value);
                            last_indicator = Some(value);
                            changed = true;
                        }

                        if changed {
                            if let (Some(battery), Some(indicator)) = (last_battery, last_indicator) {
                                let state = DeviceState { battery, indicator };
                                if let Err(e) = send_state(&socket, &state).await {
                                    eprintln!("Failed to send state: {:?}", e);
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    println!("Notification stream ended");
                    break;
                }
            }
            _ = async {
                let sock = socket.lock().await;
                sock.readable().await
            } => {
                // Check if client disconnected
                let mut buf = [0u8; 1];
                match socket.lock().await.try_read(&mut buf) {
                    Ok(0) => {
                        println!("Client disconnected");
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // Cleanup
    println!("Disconnecting from device...");
    peripheral.disconnect().await.ok();
    adapter.stop_scan().await.ok();

    Ok(())
}

async fn send_state(socket: &Arc<Mutex<TcpStream>>, state: &DeviceState) -> Result<()> {
    let json = serde_json::to_string(state)?;
    let mut sock = socket.lock().await;
    sock.write_all(json.as_bytes()).await?;
    sock.write_all(b"\n").await?;
    Ok(())
}
