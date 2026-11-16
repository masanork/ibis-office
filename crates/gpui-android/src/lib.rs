mod dispatcher;
mod display;
mod jni_bridge;
mod platform;
mod window;

pub use android_activity;
pub use jni_bridge::sync_saved_file;
pub use platform::AndroidPlatform;

pub fn init_logging() {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("ibisOffice"),
    );
    std::panic::set_hook(Box::new(|info| {
        log::error!("RUST PANIC: {info}");
    }));
}
