use anyhow::{Context, Result};
use pipewire as pw;
use pw::main_loop::MainLoopBox;
use pw::context::ContextBox;
use pw::types::ObjectType;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct AudioInputDevice {
    pub id: u32,
    pub name: String,
    pub description: String,
}

pub fn list_input_devices() -> Result<Vec<AudioInputDevice>> {
    pw::init();
    let mainloop = MainLoopBox::new(None).context("create main loop")?;
    let context = ContextBox::new(mainloop.loop_(), None).context("create context")?;
    let core = context.connect(None).context("connect core")?;
    let registry = core.get_registry().context("get registry")?;

    let devices = Arc::new(Mutex::new(Vec::<AudioInputDevice>::new()));
    let devices_clone = devices.clone();

    let _listener = registry
        .add_listener_local()
        .global(move |global| {
            if global.type_ == ObjectType::Node {
                if let Some(props) = global.props.as_ref() {
                    if let Some(media_class) = props.get("media.class") {
                        if media_class.contains("Audio/Source") {
                            let id = global.id as u32;
                            let name = props.get("node.name").unwrap_or("unknown").to_string();
                            let description = props
                                .get("node.description")
                                .or_else(|| props.get("device.description"))
                                .unwrap_or(&name)
                                .to_string();
                            devices_clone.lock().unwrap().push(AudioInputDevice { id, name, description });
                        }
                    }
                }
            }
        })
        .register();

    core.sync(0).context("core sync")?;

    // Process events briefly to allow initial global events to be handled
    use pw::loop_::Timeout;
    for _ in 0..10 {
        mainloop.loop_().iterate(Timeout::Finite(Duration::from_millis(5)));
    }

    Ok(devices.lock().unwrap().clone())
}
