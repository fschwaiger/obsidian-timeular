use anyhow::Result;
use btleplug::api::{Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use futures::SinkExt;
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, RwLock};
use tokio::time::{sleep, timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

fn tracker_tcp_port() -> u16 {
    env::var("TRACKER_TCP_PORT")
        .expect("Environment variable TRACKER_TCP_PORT must be set.")
        .parse()
        .expect("TRACKER_TCP_PORT must be a valid u16 integer.")
}

fn tracker_name_prefix() -> Option<String> {
    env::var("TRACKER_NAME_PREFIX").ok()
}

fn tracker_mac_address() -> Option<String> {
    env::var("TRACKER_MAC_ADDRESS").ok()
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

    let app_state = Arc::new(AppState {
        connected_clients: Arc::new(RwLock::new(0)),
    });

    if let Err(e) = run_tcp_server(app_state).await {
        eprintln!("TCP server error: {:?}", e);
    }
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

    let ws_stream = match accept_async(socket).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WebSocket handshake failed: {:?}", e);
            *app_state.connected_clients.write().await -= 1;
            return Err(e.into());
        }
    };

    let (ws_write, _ws_read) = ws_stream.split();
    let ws_write = Arc::new(Mutex::new(ws_write));

    match timeout(Duration::from_secs(10), find_and_connect_device()).await {
        Ok(Ok((peripheral, adapter))) => {
            println!("Device connected successfully");

            // Monitor device and send updates
            if let Err(e) = monitor_device(peripheral, adapter, ws_write.clone()).await {
                eprintln!("Monitor error: {:?}", e);
            }
        }
        Ok(Err(e)) => {
            eprintln!("Failed to connect to device: {:?}", e);
            let mut ws = ws_write.lock().await;
            ws.send(Message::Text("{\"error\":\"Device not found\"}".to_string()))
                .await
                .ok();
        }
        Err(_) => {
            println!("Timeout: Device not found within 10 seconds");
            let mut ws = ws_write.lock().await;
            ws.send(Message::Text("{\"error\":\"Timeout - device not found\"}".to_string()))
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

    let scan_filter = ScanFilter::default();
    central.start_scan(scan_filter).await?;
    sleep(Duration::from_secs(3)).await;

    let target_mac = tracker_mac_address();
    let target_prefix = tracker_name_prefix();

    if target_mac.is_none() && target_prefix.is_none() {
        anyhow::bail!("Either TRACKER_MAC_ADDRESS or TRACKER_NAME_PREFIX must be set");
    }

    // Poll for device for up to 15 seconds
    for _ in 0..60 {
        let peripherals = central.peripherals().await?;

        for p in peripherals {
            match p.properties().await {
                Ok(Some(props)) => {
                    let addr = p.address();

                    let is_match = if let Some(ref target) = target_mac {
                        let addr_str = addr.to_string().to_uppercase();
                        let target_upper = target.to_uppercase().replace("-", ":");
                        addr_str == target_upper
                    } else if let Some(ref prefix) = target_prefix {
                        props.local_name.as_ref().map_or(false, |n| n.starts_with(prefix))
                    } else {
                        false
                    };

                    if is_match {
                        let name = props.local_name.clone().unwrap_or_else(|| String::from("unknown"));
                        println!("Device found: {} ({})", name, addr);
                        central.stop_scan().await?;
                        p.connect().await?;
                        p.discover_services().await?;
                        return Ok((p, central));
                    }
                }
                Ok(None) | Err(_) => {}
            }
        }

        sleep(Duration::from_millis(250)).await;
    }

    central.stop_scan().await?;
    anyhow::bail!("Tracker device not found after scanning")
}

async fn monitor_device(
    peripheral: Peripheral,
    adapter: Adapter,
    ws_write: Arc<Mutex<futures::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>>>,
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
        send_state(&ws_write, &state).await?;
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
                            last_battery = Some(value);
                            changed = true;
                        } else if uuid_str == indicator_uuid && last_indicator != Some(value) {
                            last_indicator = Some(value);
                            changed = true;
                        }

                        if changed {
                            if let (Some(battery), Some(indicator)) = (last_battery, last_indicator) {
                                let state = DeviceState { battery, indicator };
                                if let Err(e) = send_state(&ws_write, &state).await {
                                    eprintln!("Failed to send state: {:?}", e);
                                    break;
                                }
                            }
                        }
                    }
                } else {
                    break;
                }
            }
        }
    }

    peripheral.disconnect().await.ok();
    adapter.stop_scan().await.ok();

    Ok(())
}

async fn send_state(
    ws_write: &Arc<Mutex<futures::stream::SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, Message>>>,
    state: &DeviceState,
) -> Result<()> {
    let json = serde_json::to_string(state)?;
    let mut ws = ws_write.lock().await;
    ws.send(Message::Text(json)).await?;
    Ok(())
}
