#![cfg(all(target_arch = "wasm32", feature = "webgpu"))]

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

struct Capture(Arc<AtomicUsize>);

impl Capture {
    fn on_error(&self, _: wgpu::Error) {
        panic!("unexpected uncaptured error");
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

fn register(device: &wgpu::Device, drops: &Arc<AtomicUsize>) {
    let capture = Capture(drops.clone());
    device.on_uncaptured_error(Arc::new(move |error| capture.on_error(error)));
}

#[wasm_bindgen_test]
async fn releases_handlers_on_replacement_destroy_and_last_drop() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let drops = Arc::new(AtomicUsize::new(0));

    register(&device, &drops);
    register(&device, &drops);
    assert_eq!(drops.load(Ordering::SeqCst), 1);

    let clone = device.clone();
    drop(device);
    assert_eq!(drops.load(Ordering::SeqCst), 1);

    clone.destroy();
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    clone.destroy();
    drop(clone);
    drop(queue);
    assert_eq!(drops.load(Ordering::SeqCst), 2);

    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    register(&device, &drops);
    let clone = device.clone();
    drop(device);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
    drop(clone);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    drop(queue);
}

async fn wait_for_callback(count: &AtomicUsize, name: &str) {
    for _ in 0..500 {
        if count.load(Ordering::SeqCst) == 1 {
            return;
        }
        let timeout = js_sys::Promise::new(&mut |resolve, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 20)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(timeout).await.unwrap();
    }
    panic!("{name} callback count: {}", count.load(Ordering::SeqCst));
}

#[wasm_bindgen_test]
async fn delivers_errors_and_device_loss_after_cloning() {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::BROWSER_WEBGPU,
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let adapter = instance.request_adapter(&Default::default()).await.unwrap();
    let (device, queue) = adapter.request_device(&Default::default()).await.unwrap();
    let errors = Arc::new(AtomicUsize::new(0));
    let count = errors.clone();
    device.on_uncaptured_error(Arc::new(move |_| {
        count.fetch_add(1, Ordering::SeqCst);
    }));
    let lost = Arc::new(AtomicUsize::new(0));
    let count = lost.clone();
    device.set_device_lost_callback(move |reason, _| {
        assert_eq!(reason, wgpu::DeviceLostReason::Destroyed);
        count.fetch_add(1, Ordering::SeqCst);
    });
    let clone = device.clone();
    drop(device);
    let _invalid = clone.create_buffer(&wgpu::BufferDescriptor {
        label: Some("intentional invalid usage"),
        size: 4,
        usage: wgpu::BufferUsages::empty(),
        mapped_at_creation: false,
    });
    queue.submit([]);
    wait_for_callback(&errors, "uncaptured error").await;
    clone.destroy();
    wait_for_callback(&lost, "device lost").await;
}
