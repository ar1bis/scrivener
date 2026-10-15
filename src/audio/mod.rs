pub mod devices;
pub mod volume;

use anyhow::Result;
use pipewire as pw;
use pw::{properties::properties, spa};
use pw::spa::pod::{Pod, Value, Object};
use pw::spa::param::{ParamType, audio::{AudioInfoRaw, AudioFormat}};
use pw::spa::utils::SpaTypes;
use std::sync::Arc;
use tokio::sync::mpsc;

pub type AudioSample = Vec<u8>;

pub struct PipeWireCapture {
    sample_rate: u32,
    channels: u32,
    node_id: Option<u32>,
    output_file: Option<String>,
}

impl PipeWireCapture {
    pub fn new(sample_rate: u32, channels: u32, node_id: Option<u32>, output_file: Option<String>) -> Self {
        Self { sample_rate, channels, node_id, output_file }
    }

    /// Start capture and return a receiver for raw PCM chunks.
    /// Uses PipeWire bindings via a Stream, no external pw-record.
    pub fn start(self) -> Result<mpsc::Receiver<AudioSample>> {
        let (tx, rx) = mpsc::channel(32);
        let sample_rate = self.sample_rate;
        let channels = self.channels;
        let node_id = self.node_id.map(|id| id as u32);
        let output_file = self.output_file;

        let tx = Arc::new(tx);
        let file = output_file.and_then(|p| std::fs::File::create(&p).ok());

        std::thread::spawn(move || {
            pw::init();
            if let Err(e) = run_pipewire_capture(sample_rate, channels, node_id, tx, file) {
                eprintln!("PipeWire capture error: {}", e);
            }
        });

        Ok(rx)
    }
}

fn run_pipewire_capture(
    sample_rate: u32,
    channels: u32,
    node_id: Option<u32>,
    tx: Arc<mpsc::Sender<AudioSample>>,
    _file: Option<std::fs::File>,
) -> Result<(), pw::Error> {
    use pw::main_loop::MainLoopRc;
    use pw::context::ContextRc;
    use pw::spa::pod::serialize::PodSerializer;

    let mainloop = MainLoopRc::new(None)?;
    let context = ContextRc::new(&mainloop, None)?;
    let core = context.connect_rc(None)?;

    let props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
    };

    let stream = pw::stream::StreamBox::new(&core, "scrivener-capture", props)?;

    // Prepare format params: S16LE, requested rate/channels
    let mut audio_info = AudioInfoRaw::new();
    audio_info.set_format(AudioFormat::S16LE);
    audio_info.set_rate(sample_rate);
    audio_info.set_channels(channels);

    let obj = Object {
        type_: SpaTypes::ObjectParamFormat.as_raw(),
        id: ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };

    let values = PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &Value::Object(obj),
    ).unwrap().0.into_inner();

    let mut params = [Pod::from_bytes(&values).unwrap()];

    let tx_clone = tx.clone();
    let _listener = stream
        .add_local_listener_with_user_data(())
        .param_changed(|_stream, _user_data, id, param| {
            if id == pw::spa::param::ParamType::Format.as_raw() {
                // Format negotiation happens here; avoid eprintln in TUI
                let _ = param;
            }
        })
        .process(move |stream, _user_data| {
            if let Some(mut buffer) = stream.dequeue_buffer() {
                let datas = buffer.datas_mut();
                if !datas.is_empty() {
                    if let Some(slice) = datas[0].data() {
                        let chunk = slice.to_vec();
                        if tx_clone.try_send(chunk).is_err() {
                            // Receiver dropped
                        }
                    }
                }
            }
        })
        .register()?;

    stream.connect(
        spa::utils::Direction::Input,
        node_id,
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS | pw::stream::StreamFlags::RT_PROCESS,
        &mut params,
    )?;

    mainloop.run();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn test_start() {
        let cap = PipeWireCapture::new(48000, 1, None, None);
        let mut rx = cap.start().unwrap();
        // We can't guarantee sample arrival in CI without pw-record, just ensure channel created
        let _ = rx;
    }
}

#[cfg(test)]
mod device_capture_tests {
    use super::devices::list_input_devices;
    use anyhow::Result;
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    use tokio::process::Command;
    use hound::{WavSpec, WavWriter};

    #[tokio::test]
    #[ignore]
    async fn capture_10s_per_device_to_wav() -> anyhow::Result<()> {
        let devices = list_input_devices()?;
        if devices.is_empty() {
            println!("No devices found, skipping");
            return Ok(());
        }
        let out_dir = std::path::Path::new("test_captures");
        tokio::fs::create_dir_all(out_dir).await?;
        for dev in devices {
            println!("Capturing 10s from {} id={}", dev.description, dev.id);
            let wav_path = out_dir.join(format!("device_{}_{}.wav", dev.id, sanitize(&dev.name)));
            let mut cmd = Command::new("pw-record");
            cmd.arg("--raw");
            cmd.arg("--rate").arg("48000");
            cmd.arg("--channels").arg("1");
            cmd.arg("--format").arg("s16");
            cmd.arg("--latency").arg("50ms");
            cmd.arg("--target").arg(dev.id.to_string());
            cmd.arg("-");
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::inherit());
            let mut child = cmd.spawn()?;
            let mut stdout = child.stdout.take().expect("stdout");
            let spec = WavSpec { channels: 1, sample_rate: 48000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut writer = WavWriter::create(&wav_path, spec)?;
            let mut buf = vec![0u8; 4096];
            let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);
            loop {
                tokio::select! {
                    res = stdout.read(&mut buf) => {
                        match res {
                            Ok(0) => break,
                            Ok(n) => {
                                for chunk in buf[..n].chunks_exact(2) {
                                    let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                                    writer.write_sample(s)?;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => break,
                }
            }
            writer.finalize()?;
            let _ = child.kill().await;
            println!("Wrote {}", wav_path.display());
        }
        Ok(())
    }

    fn sanitize(name: &str) -> String {
        name.chars().map(|c| if c.is_alphanumeric() || c=='-'||c=='_' {c} else {'_' }).collect()
    }
}
