# Runtime v3 step 5: main.rs — new loop

with open('src/main.rs', 'r', encoding='utf-8') as f:
    c = f.read()

# Remove build_device_runtime import
c = c.replace('use anykey_engine::runtime_builder::{self, RuntimeManager};',
              'use anykey_engine::runtime_builder::RuntimeManager;')

# Fix initial pipeline setup (line 315-318): use RuntimeManager
old_init = """        if !ids.is_empty() {
            let mut new_rt = runtime_builder::build_device_runtime(
                &pipeline.config, *ids.first().unwrap_or(&1), "");
            std::mem::swap(&mut pipeline.runtime, &mut new_rt);
            log!("Multi-device: {} device(s), {} subscribed, runtime swapped","""
new_init = """        if !ids.is_empty() {
            let first_device = *ids.first().unwrap_or(&1);
            pipeline.mapping = manager.get_mapping(&pipeline.config, first_device, "");
            pipeline.state = manager.load_domain_state(1);
            pipeline.current_domain = 1;
            pipeline.current_device = first_device;
            log!("Multi-device: {} device(s), {} subscribed, pipeline initialized","""
c = c.replace(old_init, new_init)

# Fix app-aware switch (line 549-556): use manager.get_mapping + domain check
old_app = """        if let Some(app) = sensor.try_recv() {
            if !app.is_empty() && app != current_app {
                current_app = app.clone();
                let active_key = &pipeline.current_key;
                let new_rt = manager.resolve(&pipeline.config, pipeline.current_device, &app, active_key);
                std::mem::swap(&mut pipeline.runtime, new_rt);
                pipeline.current_key = (pipeline.current_device, app);
            }
        }"""
new_app = """        if let Some(app) = sensor.try_recv() {
            if !app.is_empty() && app != current_app {
                current_app = app.clone();
                pipeline.current_app = app.clone();
                pipeline.mapping = manager.get_mapping(&pipeline.config, pipeline.current_device, &app);
            }
        }"""
c = c.replace(old_app, new_app)

# Fix per-event device routing: check domain switch
# Find "pipeline.current_device = evt.device_id" and add domain check before it
old_dev = """            pipeline.current_device = evt.device_id;

            if is_down {
                pipeline.key_down(&key_name_flt);"""
new_dev = """            // ── Domain switch detection ──
            let domain_id = pipeline.config.subscribed_devices.iter()
                .find(|d| d.runtime_device_id == Some(evt.device_id))
                .and_then(|d| d.domain_id)
                .unwrap_or(1);
            let domain_id = if domain_id == 0 { evt.device_id } else { domain_id };

            if domain_id != pipeline.current_domain {
                log!("  domain switch: {} → {}", pipeline.current_domain, domain_id);
                manager.save_domain_state(pipeline.current_domain, pipeline.state.clone());
                pipeline.state = manager.load_domain_state(domain_id);
                pipeline.current_domain = domain_id;
            }

            if pipeline.current_device != evt.device_id {
                pipeline.current_device = evt.device_id;
                // Device changed within same domain — refresh mapping
                pipeline.mapping = manager.get_mapping(&pipeline.config, evt.device_id, &current_app);
                // But keep state (same domain)
            }

            if is_down {
                pipeline.key_down(&key_name_flt);"""
c = c.replace(old_dev, new_dev)

# Fix rematch section (line 812)
old_rem = """        std::mem::swap(&mut pipeline.runtime, &mut new_rt);
        log!("Multi-device [rematch]: {} device(s), {} subscribed, runtime swapped","""
new_rem = """        let first_device = *ids.first().unwrap_or(&1);
        pipeline.mapping = manager.get_mapping(&pipeline.config, first_device, "");
        log!("Multi-device [rematch]: {} device(s), {} subscribed, pipeline reloaded","""
c = c.replace(old_rem, new_rem)

with open('src/main.rs', 'w', encoding='utf-8') as f:
    f.write(c)
print('main.rs updated')
