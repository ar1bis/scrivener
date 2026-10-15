use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use audio::{devices::list_input_devices, PipeWireCapture, volume::set_node_volume};
use client::AudioModelClient;
use config::Config;
use tui::{render, AppState};
use std::io;
use tokio::sync::mpsc;
use inquire::{Select, Text};

mod audio;
mod client;
mod config;
mod tui;

#[derive(Parser, Debug)]
#[command(name = "scrivener", about = "PipeWire audio transcription TUI")]
struct Cli {
    /// Override input device by node id
    #[arg(long)]
    device_id: Option<u32>,

    /// Interactively choose input device and save to config
    #[arg(long)]
    choose_device: bool,

    /// List available input devices
    #[arg(long)]
    list_devices: bool,

    /// Write captured PCM to file
    #[arg(long)]
    output: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.list_devices {
        let devices = list_input_devices()?;
        if devices.is_empty() {
            println!("No audio input devices found.");
        } else {
            println!("Available input devices:");
            for d in devices {
                println!("  id={}  name='{}'  description='{}'", d.id, d.name, d.description);
            }
        }
        return Ok(());
    }

    let mut config = Config::load()?;

    if cli.choose_device {
        config = prompt_device_selection(config).await?;
        config.save()?;
        println!("Device saved to config.");
        return Ok(());
    }

    if let Some(id) = cli.device_id {
        config.input_node_id = Some(id);
    }
    if let Some(out) = cli.output {
        config.output_file = Some(out);
    }

    // Load or create config, prompt for device if needed
    if config.input_node_id.is_none() {
        config = prompt_device_selection(config).await?;
    } else {
        // Ensure defaults
        if config.sample_rate == 0 { config.sample_rate = 48000; }
        if config.channels == 0 { config.channels = 1; }
    }
    config.save()?;

    // Setup TUI
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // App state
    let mut app = AppState::new();

    // Populate device list for display at startup
    if let Ok(devs) = list_input_devices() {
        app.device_items = devs.iter().map(|d| format!("{} (id: {}, {})", d.description, d.id, d.name)).collect();
        app.device_ids = devs.iter().map(|d| d.id).collect();
        if let Some(id) = config.input_node_id {
            if let Some(idx) = app.device_ids.iter().position(|&did| did == id) {
                app.picker_idx = idx;
            }
        }
        if app.device_items.is_empty() {
            app.set_status("No input devices found".to_string());
        } else if app.picker_idx >= app.device_items.len() {
            app.picker_idx = 0;
        }
    } else {
        app.set_status("Failed to list input devices".to_string());
    }

    // Audio capture
    let capture = PipeWireCapture::new(config.sample_rate, config.channels, config.input_node_id, config.output_file.clone());
    let mut audio_rx = capture.start()?;

    // Audio model client config
    let _endpoint = std::env::var("VOXTRAL_ENDPOINT").unwrap_or_else(|_| "http://localhost:8000/v1/chat/completions".into());
    let _api_key = std::env::var("OPENAI_API_KEY").ok();

    // Channels for UI updates
    let (tx_transcript, mut rx_transcript) = mpsc::channel::<String>(8);
    let (tx_level, mut rx_level) = mpsc::channel::<f32>(32);

    // Spawn transcription task
    tokio::spawn(async move {
        while let Some(pcm) = audio_rx.recv().await {
            // Compute RMS level for graph
            let level = rms_level(&pcm);
            let _ = tx_level.send(level).await;
            // Send to model
            let client_clone = AudioModelClient::new("http://localhost:8000/v1/chat/completions", None);
            match client_clone.transcribe(&pcm).await {
                Ok(text) => {
                    let _ = tx_transcript.send(text).await;
                }
                Err(e) => {
                    let _ = tx_transcript.send(format!("[error] {}", e)).await;
                }
            }
        }
    });

    // Main loop
    loop {
        terminal.draw(|f| render(f, &app))?;

        // Non-blocking input
        if event::poll(std::time::Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) => {
                    if app.show_device_picker {
                        match key.code {
                            KeyCode::Up => {
                                if app.picker_idx > 0 {
                                    app.picker_idx -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if app.picker_idx + 1 < app.device_items.len() {
                                    app.picker_idx += 1;
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(&id) = app.device_ids.get(app.picker_idx) {
                                    config.input_node_id = Some(id);
                                    if let Ok(devs) = list_input_devices() {
                                        if let Some(d) = devs.iter().find(|d| d.id == id) {
                                            config.input_node_name = Some(d.name.clone());
                                        }
                                    }
                                    config.save()?;
                                    app.set_status(format!("Device changed to id {}", id));
                                }
                                app.show_device_picker = false;
                            }
                            KeyCode::Esc => {
                                app.show_device_picker = false;
                            }
                            _ => {}
                        }
                    } else {
                        match key.code {
                            KeyCode::Char('q') => break,
                            KeyCode::Char('d') => {
                                match list_input_devices() {
                                    Ok(devs) => {
                                        app.device_items = devs.iter().map(|d| format!("{} (id: {}, {})", d.description, d.id, d.name)).collect();
                                        app.device_ids = devs.iter().map(|d| d.id).collect();
                                        app.picker_idx = if let Some(id) = config.input_node_id {
                                            app.device_ids.iter().position(|&did| did == id).unwrap_or(0)
                                        } else {
                                            0
                                        };
                                        app.show_device_picker = true;
                                    }
                                    Err(e) => {
                                        app.set_status(format!("Device list error: {}", e));
                                    }
                                }
                            }
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                app.adjust_volume(0.05);
                                let vol = app.volume;
                                if let Some(id) = config.input_node_id {
                                    let _ = set_node_volume(id, vol);
                                }
                                app.set_status(format!("Volume {:.0}%", vol * 100.0));
                            }
                            KeyCode::Char('-') => {
                                app.adjust_volume(-0.05);
                                let vol = app.volume;
                                if let Some(id) = config.input_node_id {
                                    let _ = set_node_volume(id, vol);
                                }
                                app.set_status(format!("Volume {:.0}%", vol * 100.0));
                            }
                            KeyCode::Char('m') | KeyCode::Char('M') => {
                                app.muted = !app.muted;
                                if let Some(id) = config.input_node_id {
                                    let vol = if app.muted { 0.0 } else { app.volume };
                                    let _ = set_node_volume(id, vol);
                                }
                                if app.muted {
                                    app.set_status("Muted".to_string());
                                } else {
                                    app.set_status(format!("Unmuted {:.0}%", app.volume * 100.0));
                                }
                            }
                            KeyCode::Esc => break,
                            _ => {}
                        }
                    }
                }

                _ => {}
            }
        }

        // Update state from async channel
        if let Ok(text) = tokio::time::timeout(std::time::Duration::from_millis(0), rx_transcript.recv()).await {
            if let Some(t) = text {
                app.update_transcript(t);
            }
        }
        if let Ok(level) = tokio::time::timeout(std::time::Duration::from_millis(0), rx_level.recv()).await {
            if let Some(l) = level {
                app.push_level(l);
            }
        }
    }

    // Restore terminal
    disable_raw_mode()?;

    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

async fn prompt_device_selection(mut config: Config) -> Result<Config> {
    println!("No input device configured. Scanning PipeWire for audio sources...");
    let devices = match list_input_devices() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Warning: could not list PipeWire devices: {}. Continuing with default.", e);
            return Ok(config);
        }
    };
    if devices.is_empty() {
        println!("No audio input devices found via PipeWire registry. Using default.");
        return Ok(config);
    }

    let choices: Vec<String> = devices.iter().map(|d| format!("{} (id: {}, {})", d.description, d.id, d.name)).collect();
    let selected = Select::new("Select microphone input device:", choices.clone()).prompt()?;
    let idx = choices.iter().position(|c| c == &selected).unwrap_or(0);
    let dev = &devices[idx];

    config.input_node_id = Some(dev.id);
    config.input_node_name = Some(dev.name.clone());

    // Optional sample rate/channel prompts
    let sr_str = Text::new("Sample rate [48000]").with_default(&config.sample_rate.to_string()).prompt().unwrap_or_else(|_| config.sample_rate.to_string());
    if let Ok(sr) = sr_str.parse::<u32>() { config.sample_rate = sr; }
    let ch_str = Text::new("Channels [1]").with_default(&config.channels.to_string()).prompt().unwrap_or_else(|_| config.channels.to_string());
    if let Ok(ch) = ch_str.parse::<u32>() { config.channels = ch; }

    println!("Selected device: {} (id {})", dev.description, dev.id);
    Ok(config)
}

fn rms_level(pcm: &[u8]) -> f32 {
    if pcm.len() < 2 {
        return 0.0;
    }
    let samples: Vec<i16> = pcm
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect();
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|&s| {
        let v = s as f64 / i16::MAX as f64;
        v * v
    }).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt();
    rms as f32
}

