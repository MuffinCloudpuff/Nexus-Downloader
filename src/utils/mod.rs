pub fn to_safe_filename(name: &str, fallback: &str) -> String {
    let invalid_chars = ['\\', '/', ':', '*', '?', '"', '<', '>', '|'];
    let mut safe_name = name.to_string();

    for c in invalid_chars {
        safe_name = safe_name.replace(c, "_");
    }

    let safe_name = safe_name.trim();
    if safe_name.is_empty() {
        fallback.to_string()
    } else {
        safe_name.to_string()
    }
}

pub fn get_proxies() -> Option<String> {
    // TODO: Read from config later
    None
}
