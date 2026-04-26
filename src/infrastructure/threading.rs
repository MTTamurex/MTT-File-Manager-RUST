pub fn spawn_named<F, T>(name: &str, f: F) -> std::io::Result<std::thread::JoinHandle<Option<T>>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let name_owned = name.to_string();
    std::thread::Builder::new()
        .name(name_owned.clone())
        .spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            match result {
                Ok(value) => Some(value),
                Err(payload) => {
                    log::error!(
                        "[{}] thread panicked: {}",
                        name_owned,
                        panic_message(&payload)
                    );
                    None
                }
            }
        })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}
