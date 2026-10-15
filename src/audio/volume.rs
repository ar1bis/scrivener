use anyhow::{Context, Result};
use pipewire as pw;
use pw::main_loop::MainLoopBox;
use pw::context::ContextBox;
use pw::types::ObjectType;
use pw::node::Node;
use pw::spa::param::ParamType;
use pw::spa::pod::builder::Builder;
use pw::spa::pod::Pod;
use libspa_sys::{SPA_TYPE_OBJECT_Props, SPA_PROP_volume};
use std::mem::MaybeUninit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn set_node_volume(node_id: u32, volume: f32) -> Result<()> {
    let vol = volume.clamp(0.0, 1.0);
    pw::init();

    // Leak resources so listeners can be 'static
    let mainloop = Box::leak(Box::new(MainLoopBox::new(None).context("main loop")?));
    let context = Box::leak(Box::new(ContextBox::new(mainloop.loop_(), None).context("context")?));
    let core = Box::leak(Box::new(context.connect(None).context("core")?));
    let registry = Box::leak(Box::new(core.get_registry().context("registry")?));

    let node_holder = Arc::new(Mutex::new(None::<Node>));
    let node_holder_clone = node_holder.clone();

    let registry_ptr: *const pw::registry::RegistryBox = registry;

    let _listener = registry.add_listener_local().global(move |global| {
        if global.id == node_id as u32 && global.type_ == ObjectType::Node {
            let registry = unsafe { &*registry_ptr };
            if let Ok(node) = registry.bind::<Node, _>(global) {
                *node_holder_clone.lock().unwrap() = Some(node);
            }
        }
    }).register();

    core.sync(0).context("sync")?;

    use pw::loop_::Timeout;
    for _ in 0..30 {
        mainloop.loop_().iterate(Timeout::Finite(Duration::from_millis(5)));
    }

    let node = node_holder.lock().unwrap().take().context("node not found via PipeWire registry")?;

    // Build SPA Props pod with SPA_PROP_volume
    let mut buf = Vec::with_capacity(1024);
    let mut builder = Builder::new(&mut buf);
    let mut frame = MaybeUninit::uninit();
    unsafe {
        builder.push_object(&mut frame, SPA_TYPE_OBJECT_Props, ParamType::Props.as_raw()).context("push object")?;
        builder.add_prop(SPA_PROP_volume, 0).context("add prop")?;
        builder.add_float(vol as f32).context("add float")?;
        builder.pop(&mut *frame.assume_init_mut());
    }
    let pod = Pod::from_bytes(&buf).context("build volume pod")?;
    node.set_param(ParamType::Props, 0, &pod);
    Ok(())
}
