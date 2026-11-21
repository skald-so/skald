//! TEMPORARY diagnostics: appends lines to the file `SKALD_TRACE` names.

pub fn line(text: &str) {
    let Some(path) = std::env::var_os("SKALD_TRACE") else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let mut file = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return,
    };
    use std::io::Write;
    let _ = writeln!(file, "{now} pid={} {text}", std::process::id());
}
